//! # Simple Key-Value Storage Contract
//!
//! Demonstrates all LOS SDK features:
//! - State read/write/delete
//! - Event emission
//! - Caller context
//! - CIL transfers
//! - Blake3 hashing
//!
//! ## Exported functions
//!
//! | Function       | Args                  | Description                |
//! |----------------|-----------------------|----------------------------|
//! | `init`         | owner_name            | Initialize contract        |
//! | `set`          | key, value            | Set a key-value pair       |
//! | `get`          | key                   | Get value by key           |
//! | `del`          | key                   | Delete a key               |
//! | `hash`         | data                  | Compute blake3 hash        |
//! | `donate`       | recipient, amount_cil | Transfer CIL               |
//! | `info`         | (none)                | Return contract info       |
//!
//! ## Compilation
//!
//! ```bash
//! cargo build --target wasm32-unknown-unknown --release -p los-contract-examples --bin simple_storage
//! ```

#![no_std]
#![no_main]

extern crate alloc;
extern crate los_sdk;

use alloc::format;
use los_sdk::*;

#[no_mangle]
pub extern "C" fn init() -> i32 {
    let owner = arg(0).unwrap_or_default();
    if owner.is_empty() {
        log("init: missing owner name");
        set_return_str(r#"{"success":false,"msg":"owner name required"}"#);
        return 1;
    }

    let who = caller();
    state::set_str("owner", &who);
    state::set_str("owner_name", &owner);
    state::set_u64("item_count", 0);

    event::emit("Init", &format!(
        r#"{{"owner":"{}","name":"{}"}}"#,
        who, owner
    ));

    set_return_str(&format!(
        r#"{{"success":true,"owner":"{}","contract":"{}"}}"#,
        who, self_address()
    ));
    0
}

#[no_mangle]
pub extern "C" fn set() -> i32 {
    let key = match arg(0) {
        Some(k) if !k.is_empty() => k,
        _ => {
            set_return_str(r#"{"success":false,"msg":"key required"}"#);
            return 1;
        }
    };
    let value = arg(1).unwrap_or_default();

    // Check if key is new
    let is_new = !state::exists(&format!("kv:{}", key));

    state::set_str(&format!("kv:{}", key), &value);

    if is_new {
        let count = state::get_u64("item_count");
        state::set_u64("item_count", count + 1);
    }

    event::emit("Set", &format!(
        r#"{{"key":"{}","caller":"{}"}}"#,
        key, caller()
    ));

    set_return_str(&format!(
        r#"{{"success":true,"key":"{}","new":{}}}"#,
        key, is_new
    ));
    0
}

#[no_mangle]
pub extern "C" fn get() -> i32 {
    let key = match arg(0) {
        Some(k) if !k.is_empty() => k,
        _ => {
            set_return_str(r#"{"success":false,"msg":"key required"}"#);
            return 1;
        }
    };

    match state::get_str(&format!("kv:{}", key)) {
        Some(value) => {
            set_return_str(&format!(
                r#"{{"success":true,"key":"{}","value":"{}"}}"#,
                key, value
            ));
            0
        }
        None => {
            set_return_str(&format!(
                r#"{{"success":false,"msg":"key not found: {}"}}"#,
                key
            ));
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn del() -> i32 {
    let key = match arg(0) {
        Some(k) if !k.is_empty() => k,
        _ => {
            set_return_str(r#"{"success":false,"msg":"key required"}"#);
            return 1;
        }
    };

    let existed = state::exists(&format!("kv:{}", key));
    state::del(&format!("kv:{}", key));

    if existed {
        let count = state::get_u64("item_count");
        if count > 0 {
            state::set_u64("item_count", count - 1);
        }
    }

    event::emit("Del", &format!(r#"{{"key":"{}"}}"#, key));

    set_return_str(&format!(
        r#"{{"success":true,"key":"{}","existed":{}}}"#,
        key, existed
    ));
    0
}

#[no_mangle]
pub extern "C" fn hash() -> i32 {
    let data = arg(0).unwrap_or_default();
    let h = crypto::blake3(data.as_bytes());

    // Format as hex string
    let mut hex = alloc::string::String::with_capacity(64);
    for byte in &h {
        use core::fmt::Write;
        let _ = write!(hex, "{:02x}", byte);
    }

    set_return_str(&format!(
        r#"{{"success":true,"hash":"{}"}}"#,
        hex
    ));
    0
}

#[no_mangle]
pub extern "C" fn donate() -> i32 {
    let recipient = match arg(0) {
        Some(r) if !r.is_empty() => r,
        _ => {
            set_return_str(r#"{"success":false,"msg":"recipient required"}"#);
            return 1;
        }
    };
    let amount_str = arg(1).unwrap_or_default();
    let amount: u128 = match amount_str.parse() {
        Ok(a) => a,
        Err(_) => {
            set_return_str(r#"{"success":false,"msg":"invalid amount"}"#);
            return 1;
        }
    };

    match transfer(&recipient, amount) {
        Ok(()) => {
            event::emit("Donate", &format!(
                r#"{{"to":"{}","amount":"{}","from":"{}"}}"#,
                recipient, amount, caller()
            ));
            set_return_str(&format!(
                r#"{{"success":true,"to":"{}","amount":"{}"}}"#,
                recipient, amount
            ));
            0
        }
        Err(e) => {
            set_return_str(&format!(
                r#"{{"success":false,"msg":"{}"}}"#, e
            ));
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn info() -> i32 {
    let owner = state::get_str("owner").unwrap_or_default();
    let owner_name = state::get_str("owner_name").unwrap_or_default();
    let count = state::get_u64("item_count");
    let bal = balance();
    let ts = timestamp();

    set_return_str(&format!(
        r#"{{"owner":"{}","owner_name":"{}","items":{},"balance":"{}","timestamp":{}}}"#,
        owner, owner_name, count, bal, ts
    ));
    0
}
