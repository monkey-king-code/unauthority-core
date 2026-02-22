// SPDX-License-Identifier: AGPL-3.0-only
//! # ERC20-like Token Contract
//! 
//! Standard fungible token implementation for Unauthority blockchain.
//! 
//! ## Features:
//! - Fixed supply at deployment
//! - Transfer tokens between accounts
//! - Approve and transferFrom (allowance mechanism)
//! - Balance queries
//! 
//! ## Deployment:
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown
//! los-cli deploy target/wasm32-unknown-unknown/release/token.wasm \
//!   --args '{"name":"MyToken","symbol":"MTK","total_supply":1000000}'
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenInfo {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub total_supply: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Action {
    Transfer { to: String, amount: u64 },
    Approve { spender: String, amount: u64 },
    TransferFrom { from: String, to: String, amount: u64 },
    BalanceOf { account: String },
    Allowance { owner: String, spender: String },
    TokenInfo,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub success: bool,
    pub data: Option<String>,
    pub message: String,
}

struct TokenState {
    info: TokenInfo,
    balances: HashMap<String, u64>,
    allowances: HashMap<(String, String), u64>, // (owner, spender) -> amount
}

// SAFETY NOTE: In WASM, contracts run in single-threaded environments.
// For production use with potential multi-threading, consider using:
// - std::sync::Mutex<TokenState> for thread-safety
// - once_cell::sync::Lazy for lazy initialization
// This pattern is acceptable ONLY for single-threaded WASM execution.
static mut STATE: Option<TokenState> = None;

#[allow(static_mut_refs)] // WASM is single-threaded
fn get_state() -> &'static mut TokenState {
    unsafe {
        STATE.as_mut().expect("Token not initialized")
    }
}

fn get_caller() -> String {
    // In real implementation, this would come from transaction context
    "LOS_CALLER_ADDRESS".to_string()
}

#[no_mangle]
pub extern "C" fn init(name_ptr: *const u8, name_len: usize, 
                       symbol_ptr: *const u8, symbol_len: usize,
                       total_supply: u64) {
    let name = unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(name_ptr, name_len)).to_string()
    };
    let symbol = unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(symbol_ptr, symbol_len)).to_string()
    };

    let creator = get_caller();
    let mut balances = HashMap::new();
    balances.insert(creator, total_supply);

    unsafe {
        STATE = Some(TokenState {
            info: TokenInfo {
                name,
                symbol,
                decimals: 8, // Match LOS's CIL denomination
                total_supply,
            },
            balances,
            allowances: HashMap::new(),
        });
    }
}

#[no_mangle]
pub extern "C" fn execute(input_ptr: *const u8, input_len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let action: Action = serde_json::from_slice(input).expect("Invalid input JSON");

    let caller = get_caller();
    let state = get_state();

    let response = match action {
        Action::Transfer { to, amount } => {
            let from_balance = state.balances.get(&caller).copied().unwrap_or(0);
            if from_balance < amount {
                Response {
                    success: false,
                    data: None,
                    message: "Insufficient balance".to_string(),
                }
            } else {
                *state.balances.entry(caller.clone()).or_insert(0) -= amount;
                *state.balances.entry(to.clone()).or_insert(0) += amount;
                Response {
                    success: true,
                    data: None,
                    message: format!("Transferred {} to {}", amount, to),
                }
            }
        }
        Action::Approve { spender, amount } => {
            state.allowances.insert((caller.clone(), spender.clone()), amount);
            Response {
                success: true,
                data: None,
                message: format!("Approved {} for {}", amount, spender),
            }
        }
        Action::TransferFrom { from, to, amount } => {
            let allowance = state.allowances.get(&(from.clone(), caller.clone())).copied().unwrap_or(0);
            let from_balance = state.balances.get(&from).copied().unwrap_or(0);

            if allowance < amount {
                Response {
                    success: false,
                    data: None,
                    message: "Allowance exceeded".to_string(),
                }
            } else if from_balance < amount {
                Response {
                    success: false,
                    data: None,
                    message: "Insufficient balance".to_string(),
                }
            } else {
                *state.balances.entry(from.clone()).or_insert(0) -= amount;
                *state.balances.entry(to.clone()).or_insert(0) += amount;
                *state.allowances.entry((from.clone(), caller.clone())).or_insert(0) -= amount;
                Response {
                    success: true,
                    data: None,
                    message: format!("Transferred {} from {} to {}", amount, from, to),
                }
            }
        }
        Action::BalanceOf { account } => {
            let balance = state.balances.get(&account).copied().unwrap_or(0);
            Response {
                success: true,
                data: Some(balance.to_string()),
                message: format!("Balance: {}", balance),
            }
        }
        Action::Allowance { owner, spender } => {
            let allowance = state.allowances.get(&(owner.clone(), spender.clone())).copied().unwrap_or(0);
            Response {
                success: true,
                data: Some(allowance.to_string()),
                message: format!("Allowance: {}", allowance),
            }
        }
        Action::TokenInfo => {
            Response {
                success: true,
                data: Some(serde_json::to_string(&state.info).unwrap()),
                message: "Token info retrieved".to_string(),
            }
        }
    };

    let output = serde_json::to_vec(&response).expect("Failed to serialize response");
    output.as_ptr()
}

fn main() {
    println!("ERC20-like Token Contract for LOS");
    println!("Compile to WASM before deployment");
}
