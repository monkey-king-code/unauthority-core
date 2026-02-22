// SPDX-License-Identifier: AGPL-3.0-only
//! # Hello World Smart Contract
//! 
//! Simple contract demonstrating basic storage operations on Unauthority blockchain.
//! 
//! ## Features:
//! - Store arbitrary key-value pairs
//! - Retrieve stored values
//! - Delete entries
//! 
//! ## Deployment:
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown
//! los-cli deploy target/wasm32-unknown-unknown/release/hello_world.wasm
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
pub enum Action {
    Set { key: String, value: String },
    Get { key: String },
    Delete { key: String },
    ListAll,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub success: bool,
    pub data: Option<String>,
    pub message: String,
}

// SAFETY NOTE: In WASM, contracts run in single-threaded environments.
// For production use with potential multi-threading, consider using:
// - std::sync::Mutex<HashMap<String, String>> for thread-safety
// - once_cell::sync::Lazy for lazy initialization
// This pattern is acceptable ONLY for single-threaded WASM execution.
static mut STORAGE: Option<HashMap<String, String>> = None;

#[allow(static_mut_refs)] // WASM is single-threaded
fn get_storage() -> &'static mut HashMap<String, String> {
    unsafe {
        if STORAGE.is_none() {
            STORAGE = Some(HashMap::new());
        }
        STORAGE.as_mut().unwrap()
    }
}

#[no_mangle]
pub extern "C" fn execute(input_ptr: *const u8, input_len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let action: Action = serde_json::from_slice(input).expect("Invalid input JSON");

    let storage = get_storage();
    let response = match action {
        Action::Set { key, value } => {
            storage.insert(key.clone(), value.clone());
            Response {
                success: true,
                data: None,
                message: format!("Key '{}' set to '{}'", key, value),
            }
        }
        Action::Get { key } => {
            if let Some(value) = storage.get(&key) {
                Response {
                    success: true,
                    data: Some(value.clone()),
                    message: format!("Value retrieved for key '{}'", key),
                }
            } else {
                Response {
                    success: false,
                    data: None,
                    message: format!("Key '{}' not found", key),
                }
            }
        }
        Action::Delete { key } => {
            if storage.remove(&key).is_some() {
                Response {
                    success: true,
                    data: None,
                    message: format!("Key '{}' deleted", key),
                }
            } else {
                Response {
                    success: false,
                    data: None,
                    message: format!("Key '{}' not found", key),
                }
            }
        }
        Action::ListAll => {
            let all_keys: Vec<String> = storage.keys().cloned().collect();
            Response {
                success: true,
                data: Some(serde_json::to_string(&all_keys).unwrap()),
                message: format!("Total keys: {}", all_keys.len()),
            }
        }
    };

    let output = serde_json::to_vec(&response).expect("Failed to serialize response");
    output.as_ptr()
}

#[no_mangle]
pub extern "C" fn init() {
    // Contract initialization
    let storage = get_storage();
    storage.insert("initialized".to_string(), "true".to_string());
}

fn main() {
    println!("Hello World Smart Contract");
    println!("This contract must be compiled to WASM and deployed on LOS blockchain");
}
