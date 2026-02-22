// SPDX-License-Identifier: AGPL-3.0-only
//! # USP-01: Unauthority Standard for Permissionless Tokens
//!
//! Native Fungible Token Standard for the Unauthority (LOS) blockchain.
//!
//! ## Overview
//! USP-01 defines a standardised interface for fungible tokens deployed as
//! WASM smart contracts on the Unauthority Virtual Machine (UVM).  Every
//! USP-01 token MUST expose the mandatory actions listed below so that wallets,
//! DEXs and other contracts can interact with any compliant token uniformly.
//!
//! ## Features
//! - Fixed or uncapped supply at deployment time
//! - Transfer, Approve, TransferFrom (ERC-20-like)
//! - Burn (permanent supply reduction)
//! - Wrapped Asset support (wBTC, wETH, etc.)
//! - All amounts in atomic units (`u128`) — NO floating-point
//! - Standard event types for indexing
//!
//! ## Architecture
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  USP-01 Contract (WASM)                             │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐  │
//! │  │ Metadata  │  │ Balances │  │ Allowances       │  │
//! │  │ name      │  │ addr→u128│  │ (owner,spender)  │  │
//! │  │ symbol    │  │          │  │   →u128          │  │
//! │  │ decimals  │  │          │  │                  │  │
//! │  │ supply    │  │          │  │                  │  │
//! │  └──────────┘  └──────────┘  └──────────────────┘  │
//! └─────────────────────────────────────────────────────┘
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// ─────────────────────────────────────────────────────────────
// u128 ↔ String serialization (JSON doesn't support 128-bit integers)
// ─────────────────────────────────────────────────────────────

mod u128_str {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(val: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&val.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<u128>().map_err(serde::de::Error::custom)
    }
}

// ─────────────────────────────────────────────────────────────
// TOKEN METADATA
// ─────────────────────────────────────────────────────────────

/// USP-01 Token Metadata — stored in contract state at deployment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenMetadata {
    /// Human-readable name (e.g. "Wrapped Bitcoin")
    pub name: String,
    /// Ticker symbol (e.g. "wBTC"), max 8 characters
    pub symbol: String,
    /// Decimal places for display. LOS native = 11 (CIL), typical token = 8.
    pub decimals: u8,
    /// Total supply in atomic units (fixed at deployment or mutable via bridge)
    #[serde(with = "u128_str")]
    pub total_supply: u128,
    /// If `true`, this is a Wrapped Asset backed by an external chain
    pub is_wrapped: bool,
    /// Optional: original chain (e.g. "bitcoin", "ethereum")
    #[serde(default)]
    pub wrapped_origin: String,
    /// Maximum supply cap (0 = no cap / governed by wrapping bridge)
    #[serde(default, with = "u128_str")]
    pub max_supply: u128,
}

impl TokenMetadata {
    /// Validate metadata fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.name.len() > 64 {
            return Err("Name must be 1-64 characters".to_string());
        }
        if self.symbol.is_empty() || self.symbol.len() > 8 {
            return Err("Symbol must be 1-8 characters".to_string());
        }
        if self.decimals > 18 {
            return Err("Decimals must be 0-18".to_string());
        }
        if self.total_supply == 0 {
            return Err("Total supply must be > 0".to_string());
        }
        if self.max_supply > 0 && self.total_supply > self.max_supply {
            return Err("Total supply exceeds max supply".to_string());
        }
        if self.is_wrapped && self.wrapped_origin.is_empty() {
            return Err("Wrapped tokens must specify wrapped_origin".to_string());
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────
// STANDARD ACTIONS (Contract ABI)
// ─────────────────────────────────────────────────────────────

/// Standard actions that all USP-01 contracts MUST implement.
/// The contract's `execute()` entry point receives a JSON-serialised `Usp01Action`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum Usp01Action {
    /// Initialise the token — called once at deploy-time.
    /// Creator receives `total_supply` tokens.
    Init {
        name: String,
        symbol: String,
        decimals: u8,
        #[serde(with = "u128_str")]
        total_supply: u128,
        #[serde(default)]
        is_wrapped: bool,
        #[serde(default)]
        wrapped_origin: String,
        #[serde(default, with = "u128_str")]
        max_supply: u128,
    },

    /// Transfer `amount` tokens from caller to `to`.
    Transfer {
        to: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },

    /// Approve `spender` to spend up to `amount` tokens on behalf of caller.
    Approve {
        spender: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },

    /// Transfer `amount` tokens from `from` to `to` (requires allowance).
    TransferFrom {
        from: String,
        to: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },

    /// Permanently burn `amount` tokens from caller's balance.
    /// Decreases `total_supply`.
    Burn {
        #[serde(with = "u128_str")]
        amount: u128,
    },

    // ── Read-only queries (gas cost ≤ 50) ──
    /// Return balance of `account` in atomic units.
    BalanceOf { account: String },

    /// Return allowance granted by `owner` to `spender`.
    AllowanceOf { owner: String, spender: String },

    /// Return total supply in atomic units.
    TotalSupply,

    /// Return full token metadata.
    TokenInfo,

    // ── Wrapped Asset Operations (optional, for is_wrapped = true) ──
    /// Mint wrapped tokens when a deposit is confirmed on the source chain.
    /// Only callable by the designated bridge operator / multisig.
    WrapMint {
        to: String,
        #[serde(with = "u128_str")]
        amount: u128,
        /// Proof of deposit on source chain (tx hash or oracle attestation)
        proof: String,
    },

    /// Burn wrapped tokens for redemption on the source chain.
    WrapBurn {
        #[serde(with = "u128_str")]
        amount: u128,
        /// Destination address on the source chain
        destination: String,
    },
}

// ─────────────────────────────────────────────────────────────
// STANDARD EVENTS
// ─────────────────────────────────────────────────────────────

/// Standard event types emitted by USP-01 contracts.
/// Nodes index these for wallet and explorer query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event")]
pub enum Usp01Event {
    /// Emitted on Transfer / TransferFrom
    Transfer {
        from: String,
        to: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },
    /// Emitted on Approve
    Approval {
        owner: String,
        spender: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },
    /// Emitted on Burn
    Burn {
        from: String,
        #[serde(with = "u128_str")]
        amount: u128,
    },
    /// Emitted on WrapMint
    WrapMint {
        to: String,
        #[serde(with = "u128_str")]
        amount: u128,
        proof: String,
    },
    /// Emitted on WrapBurn
    WrapBurn {
        from: String,
        #[serde(with = "u128_str")]
        amount: u128,
        destination: String,
    },
}

// ─────────────────────────────────────────────────────────────
// STANDARD RESPONSE
// ─────────────────────────────────────────────────────────────

/// Standard response from a USP-01 contract execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usp01Response {
    pub success: bool,
    /// JSON-encoded return data (balance, token info, etc.)
    #[serde(default)]
    pub data: Option<String>,
    /// Human-readable message
    pub message: String,
    /// Events emitted during this call
    #[serde(default)]
    pub events: Vec<Usp01Event>,
}

// ─────────────────────────────────────────────────────────────
// VALIDATION HELPERS (used by nodes to validate contract outputs)
// ─────────────────────────────────────────────────────────────

/// Validate a USP-01 action before forwarding to the VM.
pub fn validate_action(action: &Usp01Action) -> Result<(), String> {
    match action {
        Usp01Action::Init {
            name,
            symbol,
            decimals,
            total_supply,
            max_supply,
            ..
        } => {
            if name.is_empty() || name.len() > 64 {
                return Err("Init: name must be 1-64 chars".to_string());
            }
            if symbol.is_empty() || symbol.len() > 8 {
                return Err("Init: symbol must be 1-8 chars".to_string());
            }
            if *decimals > 18 {
                return Err("Init: decimals must be 0-18".to_string());
            }
            if *total_supply == 0 {
                return Err("Init: total_supply must be > 0".to_string());
            }
            if *max_supply > 0 && *total_supply > *max_supply {
                return Err("Init: total_supply > max_supply".to_string());
            }
            Ok(())
        }
        Usp01Action::Transfer { to, amount } => {
            if to.is_empty() {
                return Err("Transfer: recipient address is empty".to_string());
            }
            if *amount == 0 {
                return Err("Transfer: amount must be > 0".to_string());
            }
            Ok(())
        }
        Usp01Action::Approve { spender, .. } => {
            if spender.is_empty() {
                return Err("Approve: spender address is empty".to_string());
            }
            Ok(())
        }
        Usp01Action::TransferFrom { from, to, amount } => {
            if from.is_empty() || to.is_empty() {
                return Err("TransferFrom: addresses must not be empty".to_string());
            }
            if *amount == 0 {
                return Err("TransferFrom: amount must be > 0".to_string());
            }
            Ok(())
        }
        Usp01Action::Burn { amount } => {
            if *amount == 0 {
                return Err("Burn: amount must be > 0".to_string());
            }
            Ok(())
        }
        Usp01Action::BalanceOf { account } => {
            if account.is_empty() {
                return Err("BalanceOf: account is empty".to_string());
            }
            Ok(())
        }
        Usp01Action::AllowanceOf { owner, spender } => {
            if owner.is_empty() || spender.is_empty() {
                return Err("AllowanceOf: addresses must not be empty".to_string());
            }
            Ok(())
        }
        Usp01Action::TotalSupply | Usp01Action::TokenInfo => Ok(()),
        Usp01Action::WrapMint { to, amount, proof } => {
            if to.is_empty() {
                return Err("WrapMint: recipient is empty".to_string());
            }
            if *amount == 0 {
                return Err("WrapMint: amount must be > 0".to_string());
            }
            if proof.is_empty() {
                return Err("WrapMint: proof is empty".to_string());
            }
            Ok(())
        }
        Usp01Action::WrapBurn {
            amount,
            destination,
        } => {
            if *amount == 0 {
                return Err("WrapBurn: amount must be > 0".to_string());
            }
            if destination.is_empty() {
                return Err("WrapBurn: destination is empty".to_string());
            }
            Ok(())
        }
    }
}

// ─────────────────────────────────────────────────────────────
// REFERENCE IMPLEMENTATION (In-process, for testing / light nodes)
// ─────────────────────────────────────────────────────────────

/// In-memory USP-01 token state.
/// This is the reference implementation used for:
/// 1. Unit testing the standard
/// 2. Light-node simulation without WASM compilation
///
/// Production contracts MUST compile to WASM and execute inside the UVM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usp01Token {
    pub metadata: TokenMetadata,
    /// MAINNET: BTreeMap for deterministic serialization across validators
    pub balances: BTreeMap<String, u128>,
    /// (owner, spender) → allowance
    /// MAINNET: BTreeMap for deterministic serialization across validators
    pub allowances: BTreeMap<(String, String), u128>,
    /// Designated bridge operator for wrapped assets (empty = no bridge)
    pub bridge_operator: String,
    /// SECURITY FIX C-19: Set of already-used WrapMint proofs.
    /// Prevents replay attacks where the same deposit proof is submitted
    /// multiple times to mint tokens without additional backing.
    #[serde(default)]
    pub used_proofs: BTreeSet<String>,
}

impl Usp01Token {
    /// Create a new USP-01 token with initial supply assigned to `creator`.
    pub fn new(
        name: String,
        symbol: String,
        decimals: u8,
        total_supply: u128,
        creator: String,
    ) -> Result<Self, String> {
        let metadata = TokenMetadata {
            name,
            symbol,
            decimals,
            total_supply,
            is_wrapped: false,
            wrapped_origin: String::new(),
            max_supply: 0,
        };
        metadata.validate()?;

        let mut balances = BTreeMap::new();
        balances.insert(creator, total_supply);

        Ok(Self {
            metadata,
            balances,
            allowances: BTreeMap::new(),
            bridge_operator: String::new(),
            used_proofs: BTreeSet::new(),
        })
    }

    /// Create a new wrapped token (e.g. wBTC).
    pub fn new_wrapped(
        name: String,
        symbol: String,
        decimals: u8,
        wrapped_origin: String,
        bridge_operator: String,
    ) -> Result<Self, String> {
        if wrapped_origin.is_empty() {
            return Err("Wrapped token must specify origin chain".to_string());
        }
        if bridge_operator.is_empty() {
            return Err("Wrapped token must specify bridge operator".to_string());
        }

        let metadata = TokenMetadata {
            name,
            symbol,
            decimals,
            total_supply: 0, // Minted on demand via bridge
            is_wrapped: true,
            wrapped_origin,
            max_supply: 0, // No cap — supply tracks backing chain
        };

        Ok(Self {
            metadata,
            balances: BTreeMap::new(),
            allowances: BTreeMap::new(),
            bridge_operator,
            used_proofs: BTreeSet::new(),
        })
    }

    /// Execute a USP-01 action. `caller` is the verified LOS address.
    pub fn execute(&mut self, caller: &str, action: Usp01Action) -> Usp01Response {
        match action {
            Usp01Action::Init { .. } => {
                // Init is handled at deployment — reject runtime calls
                Usp01Response {
                    success: false,
                    data: None,
                    message: "Init can only be called at deployment".to_string(),
                    events: Vec::new(),
                }
            }

            Usp01Action::Transfer { to, amount } => {
                let from_balance = self.balances.get(caller).copied().unwrap_or(0);
                if from_balance < amount {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!(
                            "Insufficient balance: have {} need {}",
                            from_balance, amount
                        ),
                        events: Vec::new(),
                    };
                }
                // Debit sender (checked_sub for defense-in-depth)
                {
                    let bal = self.balances.entry(caller.to_string()).or_insert(0);
                    *bal = bal.checked_sub(amount).unwrap_or(0);
                }
                // Credit recipient (checked_add prevents u128 overflow)
                {
                    let bal = self.balances.entry(to.clone()).or_insert(0);
                    *bal = bal.checked_add(amount).unwrap_or(u128::MAX);
                }

                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Transferred {} to {}", amount, to),
                    events: vec![Usp01Event::Transfer {
                        from: caller.to_string(),
                        to,
                        amount,
                    }],
                }
            }

            Usp01Action::Approve { spender, amount } => {
                self.allowances
                    .insert((caller.to_string(), spender.clone()), amount);
                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Approved {} for {}", amount, spender),
                    events: vec![Usp01Event::Approval {
                        owner: caller.to_string(),
                        spender,
                        amount,
                    }],
                }
            }

            Usp01Action::TransferFrom { from, to, amount } => {
                let allowance = self
                    .allowances
                    .get(&(from.clone(), caller.to_string()))
                    .copied()
                    .unwrap_or(0);
                if allowance < amount {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!("Allowance exceeded: have {} need {}", allowance, amount),
                        events: Vec::new(),
                    };
                }
                let from_balance = self.balances.get(&from).copied().unwrap_or(0);
                if from_balance < amount {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!(
                            "Insufficient balance: {} has {} need {}",
                            from, from_balance, amount
                        ),
                        events: Vec::new(),
                    };
                }
                // Debit (checked_sub for defense-in-depth)
                {
                    let bal = self.balances.entry(from.clone()).or_insert(0);
                    *bal = bal.checked_sub(amount).unwrap_or(0);
                }
                // Credit (checked_add prevents u128 overflow)
                {
                    let bal = self.balances.entry(to.clone()).or_insert(0);
                    *bal = bal.checked_add(amount).unwrap_or(u128::MAX);
                }
                // Reduce allowance (checked_sub for defense-in-depth)
                {
                    let allow = self
                        .allowances
                        .entry((from.clone(), caller.to_string()))
                        .or_insert(0);
                    *allow = allow.checked_sub(amount).unwrap_or(0);
                }

                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Transferred {} from {} to {}", amount, from, to),
                    events: vec![Usp01Event::Transfer { from, to, amount }],
                }
            }

            Usp01Action::Burn { amount } => {
                let balance = self.balances.get(caller).copied().unwrap_or(0);
                if balance < amount {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!(
                            "Insufficient balance to burn: have {} need {}",
                            balance, amount
                        ),
                        events: Vec::new(),
                    };
                }
                {
                    let bal = self.balances.entry(caller.to_string()).or_insert(0);
                    *bal = bal.checked_sub(amount).unwrap_or(0);
                }
                // Decrease total supply permanently
                self.metadata.total_supply = self.metadata.total_supply.saturating_sub(amount);

                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Burned {} tokens", amount),
                    events: vec![Usp01Event::Burn {
                        from: caller.to_string(),
                        amount,
                    }],
                }
            }

            Usp01Action::BalanceOf { account } => {
                let balance = self.balances.get(&account).copied().unwrap_or(0);
                Usp01Response {
                    success: true,
                    data: Some(balance.to_string()),
                    message: format!("Balance: {}", balance),
                    events: Vec::new(),
                }
            }

            Usp01Action::AllowanceOf { owner, spender } => {
                let allowance = self
                    .allowances
                    .get(&(owner.clone(), spender.clone()))
                    .copied()
                    .unwrap_or(0);
                Usp01Response {
                    success: true,
                    data: Some(allowance.to_string()),
                    message: format!("Allowance: {}", allowance),
                    events: Vec::new(),
                }
            }

            Usp01Action::TotalSupply => Usp01Response {
                success: true,
                data: Some(self.metadata.total_supply.to_string()),
                message: "Total supply".to_string(),
                events: Vec::new(),
            },

            Usp01Action::TokenInfo => Usp01Response {
                success: true,
                data: Some(
                    serde_json::to_string(&self.metadata).unwrap_or_else(|_| "{}".to_string()),
                ),
                message: "Token info".to_string(),
                events: Vec::new(),
            },

            Usp01Action::WrapMint { to, amount, proof } => {
                if !self.metadata.is_wrapped {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: "WrapMint: token is not a wrapped asset".to_string(),
                        events: Vec::new(),
                    };
                }
                if caller != self.bridge_operator {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: "WrapMint: only bridge operator can mint".to_string(),
                        events: Vec::new(),
                    };
                }
                // SECURITY FIX C-19: Reject duplicate proofs (replay protection)
                if self.used_proofs.contains(&proof) {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!("WrapMint: proof already used (replay rejected): {}", proof),
                        events: Vec::new(),
                    };
                }
                self.used_proofs.insert(proof.clone());
                // Mint tokens (checked_add prevents u128 overflow)
                {
                    let bal = self.balances.entry(to.clone()).or_insert(0);
                    *bal = bal.checked_add(amount).unwrap_or(u128::MAX);
                }
                self.metadata.total_supply = self.metadata.total_supply.saturating_add(amount);

                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Wrapped {} to {}", amount, to),
                    events: vec![Usp01Event::WrapMint { to, amount, proof }],
                }
            }

            Usp01Action::WrapBurn {
                amount,
                destination,
            } => {
                if !self.metadata.is_wrapped {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: "WrapBurn: token is not a wrapped asset".to_string(),
                        events: Vec::new(),
                    };
                }
                let balance = self.balances.get(caller).copied().unwrap_or(0);
                if balance < amount {
                    return Usp01Response {
                        success: false,
                        data: None,
                        message: format!(
                            "WrapBurn: insufficient balance {} need {}",
                            balance, amount
                        ),
                        events: Vec::new(),
                    };
                }
                {
                    let bal = self.balances.entry(caller.to_string()).or_insert(0);
                    *bal = bal.checked_sub(amount).unwrap_or(0);
                }
                self.metadata.total_supply = self.metadata.total_supply.saturating_sub(amount);

                Usp01Response {
                    success: true,
                    data: None,
                    message: format!("Unwrap {} to {}", amount, destination),
                    events: vec![Usp01Event::WrapBurn {
                        from: caller.to_string(),
                        amount,
                        destination,
                    }],
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "LOSWalice000000000000000000000000000000";
    const BOB: &str = "LOSWbob00000000000000000000000000000000";
    const CHARLIE: &str = "LOSWcharlie0000000000000000000000000000";
    const BRIDGE: &str = "LOSWbridge00000000000000000000000000000";

    fn make_token(supply: u128) -> Usp01Token {
        Usp01Token::new(
            "TestToken".to_string(),
            "TST".to_string(),
            8,
            supply,
            ALICE.to_string(),
        )
        .unwrap()
    }

    // ── Metadata Validation ──

    #[test]
    fn test_metadata_valid() {
        let m = TokenMetadata {
            name: "Test".to_string(),
            symbol: "TST".to_string(),
            decimals: 8,
            total_supply: 1_000_000,
            is_wrapped: false,
            wrapped_origin: String::new(),
            max_supply: 0,
        };
        assert!(m.validate().is_ok());
    }

    #[test]
    fn test_metadata_empty_name() {
        let m = TokenMetadata {
            name: String::new(),
            symbol: "TST".to_string(),
            decimals: 8,
            total_supply: 1_000_000,
            is_wrapped: false,
            wrapped_origin: String::new(),
            max_supply: 0,
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_metadata_zero_supply() {
        let m = TokenMetadata {
            name: "X".to_string(),
            symbol: "X".to_string(),
            decimals: 0,
            total_supply: 0,
            is_wrapped: false,
            wrapped_origin: String::new(),
            max_supply: 0,
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_metadata_wrapped_no_origin() {
        let m = TokenMetadata {
            name: "wBTC".to_string(),
            symbol: "wBTC".to_string(),
            decimals: 8,
            total_supply: 100,
            is_wrapped: true,
            wrapped_origin: String::new(),
            max_supply: 0,
        };
        assert!(m.validate().is_err());
    }

    // ── Action Validation ──

    #[test]
    fn test_validate_transfer() {
        let a = Usp01Action::Transfer {
            to: BOB.to_string(),
            amount: 100,
        };
        assert!(validate_action(&a).is_ok());
    }

    #[test]
    fn test_validate_transfer_zero() {
        let a = Usp01Action::Transfer {
            to: BOB.to_string(),
            amount: 0,
        };
        assert!(validate_action(&a).is_err());
    }

    #[test]
    fn test_validate_transfer_empty_to() {
        let a = Usp01Action::Transfer {
            to: String::new(),
            amount: 100,
        };
        assert!(validate_action(&a).is_err());
    }

    // ── Token Operations ──

    #[test]
    fn test_create_token() {
        let token = make_token(1_000_000);
        assert_eq!(token.metadata.name, "TestToken");
        assert_eq!(token.metadata.symbol, "TST");
        assert_eq!(token.metadata.total_supply, 1_000_000);
        assert_eq!(token.balances.get(ALICE).copied().unwrap_or(0), 1_000_000);
    }

    #[test]
    fn test_transfer() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(
            ALICE,
            Usp01Action::Transfer {
                to: BOB.to_string(),
                amount: 300_000,
            },
        );
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], 700_000);
        assert_eq!(token.balances[BOB], 300_000);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(
            resp.events[0],
            Usp01Event::Transfer {
                from: ALICE.to_string(),
                to: BOB.to_string(),
                amount: 300_000,
            }
        );
    }

    #[test]
    fn test_transfer_insufficient() {
        let mut token = make_token(100);
        let resp = token.execute(
            ALICE,
            Usp01Action::Transfer {
                to: BOB.to_string(),
                amount: 200,
            },
        );
        assert!(!resp.success);
        assert!(resp.message.contains("Insufficient"));
    }

    #[test]
    fn test_approve_and_transfer_from() {
        let mut token = make_token(1_000_000);

        // Alice approves Charlie to spend 500_000
        let resp = token.execute(
            ALICE,
            Usp01Action::Approve {
                spender: CHARLIE.to_string(),
                amount: 500_000,
            },
        );
        assert!(resp.success);

        // Charlie transfers 200_000 from Alice to Bob
        let resp = token.execute(
            CHARLIE,
            Usp01Action::TransferFrom {
                from: ALICE.to_string(),
                to: BOB.to_string(),
                amount: 200_000,
            },
        );
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], 800_000);
        assert_eq!(token.balances[BOB], 200_000);

        // Remaining allowance
        let remaining = token
            .allowances
            .get(&(ALICE.to_string(), CHARLIE.to_string()))
            .copied()
            .unwrap_or(0);
        assert_eq!(remaining, 300_000);
    }

    #[test]
    fn test_transfer_from_exceeds_allowance() {
        let mut token = make_token(1_000_000);
        token.execute(
            ALICE,
            Usp01Action::Approve {
                spender: CHARLIE.to_string(),
                amount: 100,
            },
        );
        let resp = token.execute(
            CHARLIE,
            Usp01Action::TransferFrom {
                from: ALICE.to_string(),
                to: BOB.to_string(),
                amount: 200,
            },
        );
        assert!(!resp.success);
        assert!(resp.message.contains("Allowance exceeded"));
    }

    #[test]
    fn test_burn() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(ALICE, Usp01Action::Burn { amount: 100_000 });
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], 900_000);
        assert_eq!(token.metadata.total_supply, 900_000);
        assert_eq!(resp.events.len(), 1);
    }

    #[test]
    fn test_burn_insufficient() {
        let mut token = make_token(100);
        let resp = token.execute(ALICE, Usp01Action::Burn { amount: 200 });
        assert!(!resp.success);
    }

    #[test]
    fn test_balance_of() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(
            ALICE,
            Usp01Action::BalanceOf {
                account: ALICE.to_string(),
            },
        );
        assert!(resp.success);
        assert_eq!(resp.data, Some("1000000".to_string()));
    }

    #[test]
    fn test_total_supply() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(ALICE, Usp01Action::TotalSupply);
        assert!(resp.success);
        assert_eq!(resp.data, Some("1000000".to_string()));
    }

    #[test]
    fn test_token_info() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(ALICE, Usp01Action::TokenInfo);
        assert!(resp.success);
        let meta: TokenMetadata = serde_json::from_str(resp.data.as_ref().unwrap()).unwrap();
        assert_eq!(meta.name, "TestToken");
        assert_eq!(meta.symbol, "TST");
    }

    #[test]
    fn test_allowance_of() {
        let mut token = make_token(1_000_000);
        token.execute(
            ALICE,
            Usp01Action::Approve {
                spender: BOB.to_string(),
                amount: 42_000,
            },
        );
        let resp = token.execute(
            ALICE,
            Usp01Action::AllowanceOf {
                owner: ALICE.to_string(),
                spender: BOB.to_string(),
            },
        );
        assert!(resp.success);
        assert_eq!(resp.data, Some("42000".to_string()));
    }

    // ── Wrapped Asset Tests ──

    #[test]
    fn test_wrapped_token_creation() {
        let token = Usp01Token::new_wrapped(
            "Wrapped Bitcoin".to_string(),
            "wBTC".to_string(),
            8,
            "bitcoin".to_string(),
            BRIDGE.to_string(),
        )
        .unwrap();
        assert!(token.metadata.is_wrapped);
        assert_eq!(token.metadata.total_supply, 0);
    }

    #[test]
    fn test_wrap_mint() {
        let mut token = Usp01Token::new_wrapped(
            "Wrapped Bitcoin".to_string(),
            "wBTC".to_string(),
            8,
            "bitcoin".to_string(),
            BRIDGE.to_string(),
        )
        .unwrap();

        let resp = token.execute(
            BRIDGE,
            Usp01Action::WrapMint {
                to: ALICE.to_string(),
                amount: 100_000_000, // 1 wBTC
                proof: "btctx_abc123".to_string(),
            },
        );
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], 100_000_000);
        assert_eq!(token.metadata.total_supply, 100_000_000);
    }

    #[test]
    fn test_wrap_mint_unauthorized() {
        let mut token = Usp01Token::new_wrapped(
            "Wrapped Bitcoin".to_string(),
            "wBTC".to_string(),
            8,
            "bitcoin".to_string(),
            BRIDGE.to_string(),
        )
        .unwrap();

        // Alice (not bridge) tries to mint
        let resp = token.execute(
            ALICE,
            Usp01Action::WrapMint {
                to: ALICE.to_string(),
                amount: 999,
                proof: "fake".to_string(),
            },
        );
        assert!(!resp.success);
        assert!(resp.message.contains("bridge operator"));
    }

    #[test]
    fn test_wrap_burn() {
        let mut token = Usp01Token::new_wrapped(
            "Wrapped Ether".to_string(),
            "wETH".to_string(),
            18,
            "ethereum".to_string(),
            BRIDGE.to_string(),
        )
        .unwrap();

        // Bridge mints
        token.execute(
            BRIDGE,
            Usp01Action::WrapMint {
                to: ALICE.to_string(),
                amount: 1_000_000,
                proof: "ethtx_def456".to_string(),
            },
        );

        // Alice unwraps
        let resp = token.execute(
            ALICE,
            Usp01Action::WrapBurn {
                amount: 500_000,
                destination: "0xABC123".to_string(),
            },
        );
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], 500_000);
        assert_eq!(token.metadata.total_supply, 500_000);
    }

    #[test]
    fn test_wrap_burn_non_wrapped() {
        let mut token = make_token(1_000_000);
        let resp = token.execute(
            ALICE,
            Usp01Action::WrapBurn {
                amount: 100,
                destination: "somewhere".to_string(),
            },
        );
        assert!(!resp.success);
        assert!(resp.message.contains("not a wrapped asset"));
    }

    // ── Serialization ──

    #[test]
    fn test_action_json_roundtrip() {
        let action = Usp01Action::Transfer {
            to: BOB.to_string(),
            amount: 42_000,
        };
        let json = serde_json::to_string(&action).unwrap();
        let decoded: Usp01Action = serde_json::from_str(&json).unwrap();
        if let Usp01Action::Transfer { to, amount } = decoded {
            assert_eq!(to, BOB);
            assert_eq!(amount, 42_000);
        } else {
            panic!("Wrong variant");
        }
    }

    #[test]
    fn test_event_json_roundtrip() {
        let event = Usp01Event::Transfer {
            from: ALICE.to_string(),
            to: BOB.to_string(),
            amount: 100,
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Usp01Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn test_u128_amounts() {
        // Ensure u128 handles large token supplies without overflow
        let big_supply: u128 = 21_936_236 * 100_000_000_000; // 21.9M LOS in CIL
        let mut token = make_token(big_supply);
        let resp = token.execute(
            ALICE,
            Usp01Action::Transfer {
                to: BOB.to_string(),
                amount: 10_000_000_000_000, // 100 LOS worth
            },
        );
        assert!(resp.success);
        assert_eq!(token.balances[ALICE], big_supply - 10_000_000_000_000);
        assert_eq!(token.balances[BOB], 10_000_000_000_000);
    }
}
