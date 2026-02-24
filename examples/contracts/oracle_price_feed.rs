// SPDX-License-Identifier: AGPL-3.0-only
//! # Oracle Price Feed Consumer Contract
//! 
//! Smart contract demonstrating integration with LOS's decentralized oracle system.
//! 
//! ## Features:
//! - Fetch BTC/USD and ETH/USD prices from oracle
//! - Store historical price data
//! - Calculate average prices
//! - Emit price change events
//! 
//! ## Deployment:
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown
//! los-cli deploy target/wasm32-unknown-unknown/release/oracle_price_feed.wasm
//! ```

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PriceData {
    pub asset: String,         // "BTC" or "ETH"
    pub price_usd: u64,        // Price in cents (e.g., 4567800 = $45,678.00)
    pub timestamp: u64,        // Unix timestamp
    pub confidence: u8,        // Confidence level (0-100)
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Action {
    FetchPrice { asset: String },
    GetLatestPrice { asset: String },
    GetAveragePrice { asset: String, periods: usize },
    GetPriceHistory { asset: String, limit: usize },
    Subscribe { asset: String, threshold_percent: u8 }, // Alert if price changes > X%
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub success: bool,
    pub data: Option<String>,
    pub message: String,
}

struct OracleState {
    btc_prices: VecDeque<PriceData>,
    eth_prices: VecDeque<PriceData>,
    max_history: usize,
}

// SAFETY NOTE: In WASM, contracts run in single-threaded environments.
// For production use with potential multi-threading, consider using:
// - std::sync::Mutex<OracleState> for thread-safety
// - once_cell::sync::Lazy for lazy initialization
// This pattern is acceptable ONLY for single-threaded WASM execution.
static mut STATE: Option<OracleState> = None;

#[allow(static_mut_refs)] // WASM is single-threaded
fn get_state() -> &'static mut OracleState {
    unsafe {
        if STATE.is_none() {
            STATE = Some(OracleState {
                btc_prices: VecDeque::new(),
                eth_prices: VecDeque::new(),
                max_history: 100, // Keep last 100 price updates
            });
        }
        STATE.as_mut().unwrap()
    }
}

// Mock oracle call (in real implementation, this would call host function)
fn fetch_from_oracle(asset: &str) -> Result<PriceData, String> {
    // In production, this would be:
    // let price_bytes = unsafe { host_oracle_fetch(asset.as_ptr(), asset.len()) };
    // serde_json::from_slice(&price_bytes)
    
    // Mock data for demonstration
    match asset {
        "BTC" => Ok(PriceData {
            asset: "BTC".to_string(),
            price_usd: 4567800, // $45,678.00
            timestamp: 1738713600, // Feb 5, 2026
            confidence: 95,
        }),
        "ETH" => Ok(PriceData {
            asset: "ETH".to_string(),
            price_usd: 289500, // $2,895.00
            timestamp: 1738713600,
            confidence: 93,
        }),
        _ => Err(format!("Unsupported asset: {}", asset)),
    }
}

#[no_mangle]
pub extern "C" fn init() {
    let _state = get_state();
    // Initialize with current prices
}

#[no_mangle]
pub extern "C" fn execute(input_ptr: *const u8, input_len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let action: Action = match serde_json::from_slice(input) {
        Ok(a) => a,
        Err(e) => return error_response(&format!("Invalid input JSON: {}", e)),
    };

    let state = get_state();

    let response = match action {
        Action::FetchPrice { asset } => {
            match fetch_from_oracle(&asset) {
                Ok(price_data) => {
                    // Store in history
                    let history = match asset.as_str() {
                        "BTC" => &mut state.btc_prices,
                        "ETH" => &mut state.eth_prices,
                        _ => {
                            return error_response(&format!("Unsupported asset: {}", asset));
                        }
                    };

                    history.push_back(price_data.clone());
                    if history.len() > state.max_history {
                        history.pop_front();
                    }

                    Response {
                        success: true,
                        data: serde_json::to_string(&price_data).ok(),
                        message: format!("{} price updated: ${}.{:02}", 
                                       asset, 
                                       price_data.price_usd / 100,
                                       price_data.price_usd % 100),
                    }
                }
                Err(e) => Response {
                    success: false,
                    data: None,
                    message: e,
                }
            }
        }
        Action::GetLatestPrice { asset } => {
            let history = match asset.as_str() {
                "BTC" => &state.btc_prices,
                "ETH" => &state.eth_prices,
                _ => return error_response(&format!("Unsupported asset: {}", asset)),
            };

            if let Some(latest) = history.back() {
                Response {
                    success: true,
                    data: serde_json::to_string(latest).ok(),
                    message: format!("Latest {} price: ${}.{:02}", 
                                   asset,
                                   latest.price_usd / 100,
                                   latest.price_usd % 100),
                }
            } else {
                Response {
                    success: false,
                    data: None,
                    message: format!("No price data available for {}", asset),
                }
            }
        }
        Action::GetAveragePrice { asset, periods } => {
            let history = match asset.as_str() {
                "BTC" => &state.btc_prices,
                "ETH" => &state.eth_prices,
                _ => return error_response(&format!("Unsupported asset: {}", asset)),
            };

            let count = std::cmp::min(periods, history.len());
            if count == 0 {
                return error_response("No price data available");
            }

            let sum: u64 = history.iter().rev().take(count).map(|p| p.price_usd).sum();
            let average = sum / count as u64;

            Response {
                success: true,
                data: Some(average.to_string()),
                message: format!("Average {} price (last {} periods): ${}.{:02}",
                               asset, count, average / 100, average % 100),
            }
        }
        Action::GetPriceHistory { asset, limit } => {
            let history = match asset.as_str() {
                "BTC" => &state.btc_prices,
                "ETH" => &state.eth_prices,
                _ => return error_response(&format!("Unsupported asset: {}", asset)),
            };

            let count = std::cmp::min(limit, history.len());
            let recent: Vec<&PriceData> = history.iter().rev().take(count).collect();

            Response {
                success: true,
                data: serde_json::to_string(&recent).ok(),
                message: format!("Retrieved {} price records for {}", recent.len(), asset),
            }
        }
        Action::Subscribe { asset, threshold_percent } => {
            // In production, this would register a callback/event
            Response {
                success: true,
                data: None,
                message: format!("Subscribed to {} price alerts (threshold: {}%)", 
                               asset, threshold_percent),
            }
        }
    };

    let output = match serde_json::to_vec(&response) {
        Ok(v) => v,
        Err(_) => return error_response("Internal: failed to serialize response"),
    };
    let ptr = output.as_ptr();
    std::mem::forget(output); // Prevent deallocation — WASM host owns this memory
    ptr
}

fn error_response(message: &str) -> *const u8 {
    let response = Response {
        success: false,
        data: None,
        message: message.to_string(),
    };
    // Use unwrap_or for fallback — error_response is already the fallback path
    let output = serde_json::to_vec(&response).unwrap_or_else(|_| b"{\"success\":false}".to_vec());
    let ptr = output.as_ptr();
    std::mem::forget(output); // Prevent deallocation — WASM host owns this memory
    ptr
}

fn main() {
    println!("Oracle Price Feed Consumer Contract");
    println!("Integrates with LOS's decentralized oracle system");
}
