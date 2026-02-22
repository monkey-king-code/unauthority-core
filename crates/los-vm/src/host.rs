//! # UVM Host Functions
//!
//! Provides the bridge between WASM guest contracts and the LOS blockchain runtime.
//! Contracts compiled with `los-sdk` call these functions via WASM imports (module "env").
//!
//! ## Host Function ABI (WASM perspective)
//!
//! | Function                     | Signature (WASM types)                              | Description                          |
//! |------------------------------|------------------------------------------------------|--------------------------------------|
//! | `host_log`                   | `(i32, i32) -> ()`                                   | Debug log                            |
//! | `host_abort`                 | `(i32, i32) -> ()`                                   | Abort + revert state                 |
//! | `host_set_state`             | `(i32, i32, i32, i32) -> ()`                         | Write key-value to contract state    |
//! | `host_get_state`             | `(i32, i32, i32, i32) -> i32`                        | Read state (-1 = not found)          |
//! | `host_del_state`             | `(i32, i32) -> ()`                                   | Delete state key                     |
//! | `host_emit_event`            | `(i32, i32, i32, i32) -> ()`                         | Emit event (type + JSON data)        |
//! | `host_transfer`              | `(i32, i32, i64, i64) -> i32`                        | Transfer CIL (0=ok, 1=insuf, 2=err) |
//! | `host_get_caller`            | `(i32, i32) -> i32`                                  | Get caller address                   |
//! | `host_get_self_address`      | `(i32, i32) -> i32`                                  | Get contract address                 |
//! | `host_get_balance_lo`        | `() -> i64`                                          | Balance lower 64 bits                |
//! | `host_get_balance_hi`        | `() -> i64`                                          | Balance upper 64 bits                |
//! | `host_get_timestamp`         | `() -> i64`                                          | Block timestamp                      |
//! | `host_get_arg_count`         | `() -> i32`                                          | Number of call arguments             |
//! | `host_get_arg`               | `(i32, i32, i32) -> i32`                             | Get argument by index                |
//! | `host_set_return`            | `(i32, i32) -> ()`                                   | Set return data                      |
//! | `host_blake3`                | `(i32, i32, i32) -> i32`                             | Compute blake3 hash (32 bytes)       |

use crate::ContractEvent;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use wasmer::{imports, Function, FunctionEnv, FunctionEnvMut, Imports, Memory, Store};

// ─────────────────────────────────────────────────────────────────
// Limits (prevent abuse from malicious contracts)
// ─────────────────────────────────────────────────────────────────

/// Maximum size of a single state value (256 KB)
const MAX_STATE_VALUE_SIZE: u32 = 262_144;
/// Maximum state key length (1 KB)
const MAX_STATE_KEY_SIZE: u32 = 1_024;
/// Maximum size of return data (256 KB)
const MAX_RETURN_DATA_SIZE: u32 = 262_144;
/// Maximum size of a single log message (4 KB)
const MAX_LOG_SIZE: u32 = 4_096;
/// Maximum number of events per execution
const MAX_EVENTS: usize = 256;
/// Maximum number of transfers per execution
const MAX_TRANSFERS: usize = 64;
/// Maximum number of distinct state keys modified per execution
const MAX_STATE_KEYS: usize = 1_024;
/// Maximum number of log lines per execution
const MAX_LOGS: usize = 256;

// ─────────────────────────────────────────────────────────────────
// Shared state types
// ─────────────────────────────────────────────────────────────────

/// Host environment stored in wasmer's `FunctionEnv`.
/// Contains a reference to WASM linear memory and shared mutable data.
///
/// SAFETY: All fields are `Send + 'static` as required by wasmer.
/// `Memory` is a lightweight handle into the Store (Send).
/// `Arc<Mutex<HostData>>` is Send + Sync via Arc.
pub struct HostState {
    /// Reference to the guest's linear memory. Set after instantiation.
    pub memory: Option<Memory>,
    /// Shared mutable data accessed by host functions during execution.
    pub inner: Arc<Mutex<HostData>>,
}

/// Mutable data accessed by host functions during a single WASM execution.
///
/// All mutations here are transactional: on success, `dirty_keys` + `state`
/// are used to extract state changes. On abort/error, everything is discarded.
pub struct HostData {
    /// Working copy of contract state (original + modifications during execution).
    /// Keys are UTF-8 strings, values are raw bytes.
    pub state: BTreeMap<String, Vec<u8>>,
    /// Keys that were modified (set or deleted) during execution.
    pub dirty_keys: HashSet<String>,
    /// Events emitted during execution.
    pub events: Vec<ContractEvent>,
    /// Pending transfer requests: (recipient_address, amount_cil).
    pub transfers: Vec<(String, u128)>,
    /// Caller's LOS address (injected by node from block signature).
    pub caller: String,
    /// Contract's own address.
    pub self_address: String,
    /// Contract balance in CIL (decremented locally on transfer).
    pub balance: u128,
    /// Block timestamp (seconds since epoch).
    pub timestamp: u64,
    /// Function arguments (strings passed by the caller via REST/gossip).
    pub args: Vec<String>,
    /// Return value buffer (set by contract via `host_set_return`).
    pub return_data: Vec<u8>,
    /// Debug log lines.
    pub logs: Vec<String>,
    /// Whether contract called `host_abort`. If true, all state changes are reverted.
    pub aborted: bool,
    /// Human-readable abort reason.
    pub abort_message: String,
}

/// Result of hosted WASM execution, returned to the caller.
pub struct HostExecResult {
    /// WASM function return value. In SDK mode: 0 = success, non-zero = error code.
    /// In legacy mode: the actual computation result.
    pub return_code: i32,
    /// Return data buffer (set by contract via `host_set_return`).
    pub return_data: Vec<u8>,
    /// Total gas consumed (compilation + execution).
    pub gas_used: u64,
    /// State changes (only dirty keys). Key → new value bytes.
    pub state_changes: BTreeMap<String, Vec<u8>>,
    /// Events emitted during execution.
    pub events: Vec<ContractEvent>,
    /// Pending transfers (recipient, amount_cil).
    pub transfers: Vec<(String, u128)>,
    /// Debug logs.
    pub logs: Vec<String>,
    /// Whether contract called abort.
    pub aborted: bool,
    /// Abort reason.
    pub abort_message: String,
    /// True if the contract was called in SDK mode (no WASM-level params).
    /// False if legacy mode (WASM function has i32 params).
    pub sdk_mode: bool,
}

// ─────────────────────────────────────────────────────────────────
// Memory helpers (read/write WASM linear memory)
// ─────────────────────────────────────────────────────────────────

/// Read `len` bytes from WASM linear memory at `ptr`. Returns None on error.
fn read_guest_bytes(env: &FunctionEnvMut<HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    let memory = env.data().memory.as_ref()?;
    let view = memory.view(&env);
    let mut buf = vec![0u8; len as usize];
    view.read(ptr as u64, &mut buf).ok()?;
    Some(buf)
}

/// Read a UTF-8 string from WASM linear memory. Returns None on invalid UTF-8 or memory error.
fn read_guest_string(env: &FunctionEnvMut<HostState>, ptr: u32, len: u32) -> Option<String> {
    let bytes = read_guest_bytes(env, ptr, len)?;
    String::from_utf8(bytes).ok()
}

/// Write bytes to WASM linear memory at `ptr`, capped by `max_len`.
/// Returns number of bytes actually written, or -1 on error.
fn write_guest_bytes(env: &FunctionEnvMut<HostState>, ptr: u32, data: &[u8], max_len: u32) -> i32 {
    let memory = match env.data().memory.as_ref() {
        Some(m) => m,
        None => return -1,
    };
    let view = memory.view(&env);
    let write_len = data.len().min(max_len as usize);
    if write_len == 0 {
        return 0;
    }
    match view.write(ptr as u64, &data[..write_len]) {
        Ok(()) => write_len as i32,
        Err(_) => -1,
    }
}

// ─────────────────────────────────────────────────────────────────
// Host function implementations
// ─────────────────────────────────────────────────────────────────

/// `host_log(ptr: i32, len: i32)` — Write a debug log line.
/// Gas-metered by wasmer instruction counting. Capped at MAX_LOG_SIZE bytes.
fn host_log_fn(env: FunctionEnvMut<HostState>, ptr: i32, len: i32) {
    let len = (len as u32).min(MAX_LOG_SIZE);
    if let Some(msg) = read_guest_string(&env, ptr as u32, len) {
        if let Ok(mut inner) = env.data().inner.lock() {
            if inner.logs.len() < MAX_LOGS {
                inner.logs.push(msg);
            }
        }
    }
}

/// `host_abort(ptr: i32, len: i32)` — Set abort flag. The SDK calls `unreachable` after
/// this returns, causing a WASM trap that unwinds execution.
/// All state changes are discarded on abort.
fn host_abort_fn(env: FunctionEnvMut<HostState>, ptr: i32, len: i32) {
    let len = (len as u32).min(MAX_LOG_SIZE);
    let msg = read_guest_string(&env, ptr as u32, len).unwrap_or_default();
    if let Ok(mut inner) = env.data().inner.lock() {
        inner.aborted = true;
        inner.abort_message = msg;
    }
}

/// `host_set_state(key_ptr, key_len, val_ptr, val_len)` — Write a key-value pair to
/// the contract's persistent state. Overwrites existing values.
fn host_set_state_fn(
    env: FunctionEnvMut<HostState>,
    key_ptr: i32,
    key_len: i32,
    val_ptr: i32,
    val_len: i32,
) {
    let key_len = (key_len as u32).min(MAX_STATE_KEY_SIZE);
    let val_len = (val_len as u32).min(MAX_STATE_VALUE_SIZE);

    let key = match read_guest_string(&env, key_ptr as u32, key_len) {
        Some(k) => k,
        None => return,
    };
    let val = match read_guest_bytes(&env, val_ptr as u32, val_len) {
        Some(v) => v,
        None => return,
    };

    if let Ok(mut inner) = env.data().inner.lock() {
        // Rate-limit: max distinct keys per execution
        if inner.dirty_keys.len() >= MAX_STATE_KEYS && !inner.dirty_keys.contains(&key) {
            return;
        }
        inner.state.insert(key.clone(), val);
        inner.dirty_keys.insert(key);
    }
}

/// `host_get_state(key_ptr, key_len, out_ptr, out_max) -> i32`
/// Read a value from the contract's state. Returns actual byte length, or -1 if key not found.
/// If actual length exceeds `out_max`, data is truncated.
fn host_get_state_fn(
    env: FunctionEnvMut<HostState>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_max: i32,
) -> i32 {
    let key_len = (key_len as u32).min(MAX_STATE_KEY_SIZE);
    let key = match read_guest_string(&env, key_ptr as u32, key_len) {
        Some(k) => k,
        None => return -1,
    };

    let data = {
        let inner = match env.data().inner.lock() {
            Ok(i) => i,
            Err(_) => return -1,
        };
        match inner.state.get(&key) {
            Some(v) => v.clone(),
            None => return -1,
        }
    };

    write_guest_bytes(&env, out_ptr as u32, &data, out_max as u32)
}

/// `host_del_state(key_ptr, key_len)` — Delete a key from the contract's state.
fn host_del_state_fn(env: FunctionEnvMut<HostState>, key_ptr: i32, key_len: i32) {
    let key_len = (key_len as u32).min(MAX_STATE_KEY_SIZE);
    let key = match read_guest_string(&env, key_ptr as u32, key_len) {
        Some(k) => k,
        None => return,
    };
    if let Ok(mut inner) = env.data().inner.lock() {
        inner.state.remove(&key);
        inner.dirty_keys.insert(key); // Mark as changed (deletion)
    }
}

/// `host_emit_event(type_ptr, type_len, data_ptr, data_len)` — Emit a structured event.
/// `data` is JSON: `{"key1":"val1","key2":"val2"}`.
fn host_emit_event_fn(
    env: FunctionEnvMut<HostState>,
    type_ptr: i32,
    type_len: i32,
    data_ptr: i32,
    data_len: i32,
) {
    let type_len = (type_len as u32).min(256);
    let data_len = (data_len as u32).min(MAX_STATE_VALUE_SIZE);

    let event_type = match read_guest_string(&env, type_ptr as u32, type_len) {
        Some(t) => t,
        None => return,
    };
    let data_str = match read_guest_string(&env, data_ptr as u32, data_len) {
        Some(d) => d,
        None => return,
    };

    // Parse event data as JSON key-value pairs (gracefully defaults to empty on parse errors)
    let data: BTreeMap<String, String> = serde_json::from_str(&data_str).unwrap_or_default();

    if let Ok(mut inner) = env.data().inner.lock() {
        if inner.events.len() >= MAX_EVENTS {
            return;
        }
        let contract_addr = inner.self_address.clone();
        let ts = inner.timestamp;
        inner.events.push(ContractEvent {
            contract: contract_addr,
            event_type,
            data,
            timestamp: ts,
        });
    }
}

/// `host_transfer(addr_ptr, addr_len, amount_lo: i64, amount_hi: i64) -> i32`
/// Request a CIL transfer from the contract to `recipient`.
/// `amount` is reconstructed as `(amount_hi << 64) | amount_lo` (u128).
/// Returns: 0 = success, 1 = insufficient balance, 2 = invalid address, 3 = too many transfers.
fn host_transfer_fn(
    env: FunctionEnvMut<HostState>,
    addr_ptr: i32,
    addr_len: i32,
    amount_lo: i64,
    amount_hi: i64,
) -> i32 {
    let addr_len = (addr_len as u32).min(256);
    let recipient = match read_guest_string(&env, addr_ptr as u32, addr_len) {
        Some(a) if !a.is_empty() => a,
        _ => return 2, // Invalid address
    };

    // Reconstruct u128 from two i64 halves (reinterpreted as unsigned)
    let amount = ((amount_hi as u64 as u128) << 64) | (amount_lo as u64 as u128);
    if amount == 0 {
        return 0; // Zero transfer is a no-op
    }

    if let Ok(mut inner) = env.data().inner.lock() {
        if inner.transfers.len() >= MAX_TRANSFERS {
            return 3;
        }
        if inner.balance < amount {
            return 1; // Insufficient balance
        }
        inner.balance -= amount;
        inner.transfers.push((recipient, amount));
        0
    } else {
        2 // Lock failure treated as error
    }
}

/// `host_get_caller(out_ptr, out_max) -> i32` — Write caller's LOS address to guest memory.
/// Returns number of bytes written, or -1 on error.
fn host_get_caller_fn(env: FunctionEnvMut<HostState>, out_ptr: i32, out_max: i32) -> i32 {
    let caller = {
        let inner = match env.data().inner.lock() {
            Ok(i) => i,
            Err(_) => return -1,
        };
        inner.caller.clone()
    };
    write_guest_bytes(&env, out_ptr as u32, caller.as_bytes(), out_max as u32)
}

/// `host_get_self_address(out_ptr, out_max) -> i32` — Write contract's own address to guest memory.
/// Returns number of bytes written, or -1 on error.
fn host_get_self_address_fn(env: FunctionEnvMut<HostState>, out_ptr: i32, out_max: i32) -> i32 {
    let addr = {
        let inner = match env.data().inner.lock() {
            Ok(i) => i,
            Err(_) => return -1,
        };
        inner.self_address.clone()
    };
    write_guest_bytes(&env, out_ptr as u32, addr.as_bytes(), out_max as u32)
}

/// `host_get_balance_lo() -> i64` — Lower 64 bits of the contract's CIL balance.
fn host_get_balance_lo_fn(env: FunctionEnvMut<HostState>) -> i64 {
    let inner = match env.data().inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    (inner.balance & 0xFFFF_FFFF_FFFF_FFFF) as i64
}

/// `host_get_balance_hi() -> i64` — Upper 64 bits of the contract's CIL balance.
fn host_get_balance_hi_fn(env: FunctionEnvMut<HostState>) -> i64 {
    let inner = match env.data().inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    (inner.balance >> 64) as i64
}

/// `host_get_timestamp() -> i64` — Block timestamp in seconds since UNIX epoch.
fn host_get_timestamp_fn(env: FunctionEnvMut<HostState>) -> i64 {
    let inner = match env.data().inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    inner.timestamp as i64
}

/// `host_get_arg_count() -> i32` — Number of string arguments passed to this call.
fn host_get_arg_count_fn(env: FunctionEnvMut<HostState>) -> i32 {
    let inner = match env.data().inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    inner.args.len() as i32
}

/// `host_get_arg(idx, out_ptr, out_max) -> i32` — Get argument by index.
/// Returns byte length of the argument, or -1 if index is out of bounds.
fn host_get_arg_fn(env: FunctionEnvMut<HostState>, idx: i32, out_ptr: i32, out_max: i32) -> i32 {
    let arg_data = {
        let inner = match env.data().inner.lock() {
            Ok(i) => i,
            Err(_) => return -1,
        };
        match inner.args.get(idx as usize) {
            Some(a) => a.clone(),
            None => return -1,
        }
    };
    write_guest_bytes(&env, out_ptr as u32, arg_data.as_bytes(), out_max as u32)
}

/// `host_set_return(ptr, len)` — Set the contract's return data.
/// Called by the contract to return structured data (e.g., JSON response).
fn host_set_return_fn(env: FunctionEnvMut<HostState>, ptr: i32, len: i32) {
    let len = (len as u32).min(MAX_RETURN_DATA_SIZE);
    if let Some(data) = read_guest_bytes(&env, ptr as u32, len) {
        if let Ok(mut inner) = env.data().inner.lock() {
            inner.return_data = data;
        }
    }
}

/// `host_blake3(data_ptr, data_len, out_ptr) -> i32`
/// Compute blake3 hash of input data, write 32 bytes to `out_ptr`.
/// Returns 32 on success, -1 on error.
fn host_blake3_fn(
    env: FunctionEnvMut<HostState>,
    data_ptr: i32,
    data_len: i32,
    out_ptr: i32,
) -> i32 {
    let data_len = (data_len as u32).min(MAX_STATE_VALUE_SIZE);
    let data = match read_guest_bytes(&env, data_ptr as u32, data_len) {
        Some(d) => d,
        None => return -1,
    };
    let hash = blake3::hash(&data);
    write_guest_bytes(&env, out_ptr as u32, hash.as_bytes(), 32)
}

// ─────────────────────────────────────────────────────────────────
// Import object construction
// ─────────────────────────────────────────────────────────────────

/// Create wasmer `Imports` containing all LOS host functions.
///
/// Must be called on the same thread that owns the `Store` (inside the execution thread).
/// The returned imports are provided to `Instance::new()`. Extra imports (not required by
/// the module) are silently ignored by wasmer.
pub fn create_host_imports(store: &mut Store, env: &FunctionEnv<HostState>) -> Imports {
    let f_log = Function::new_typed_with_env(store, env, host_log_fn);
    let f_abort = Function::new_typed_with_env(store, env, host_abort_fn);
    let f_set_state = Function::new_typed_with_env(store, env, host_set_state_fn);
    let f_get_state = Function::new_typed_with_env(store, env, host_get_state_fn);
    let f_del_state = Function::new_typed_with_env(store, env, host_del_state_fn);
    let f_emit_event = Function::new_typed_with_env(store, env, host_emit_event_fn);
    let f_transfer = Function::new_typed_with_env(store, env, host_transfer_fn);
    let f_get_caller = Function::new_typed_with_env(store, env, host_get_caller_fn);
    let f_get_self = Function::new_typed_with_env(store, env, host_get_self_address_fn);
    let f_bal_lo = Function::new_typed_with_env(store, env, host_get_balance_lo_fn);
    let f_bal_hi = Function::new_typed_with_env(store, env, host_get_balance_hi_fn);
    let f_timestamp = Function::new_typed_with_env(store, env, host_get_timestamp_fn);
    let f_arg_count = Function::new_typed_with_env(store, env, host_get_arg_count_fn);
    let f_get_arg = Function::new_typed_with_env(store, env, host_get_arg_fn);
    let f_set_return = Function::new_typed_with_env(store, env, host_set_return_fn);
    let f_blake3 = Function::new_typed_with_env(store, env, host_blake3_fn);

    imports! {
        "env" => {
            "host_log" => f_log,
            "host_abort" => f_abort,
            "host_set_state" => f_set_state,
            "host_get_state" => f_get_state,
            "host_del_state" => f_del_state,
            "host_emit_event" => f_emit_event,
            "host_transfer" => f_transfer,
            "host_get_caller" => f_get_caller,
            "host_get_self_address" => f_get_self,
            "host_get_balance_lo" => f_bal_lo,
            "host_get_balance_hi" => f_bal_hi,
            "host_get_timestamp" => f_timestamp,
            "host_get_arg_count" => f_arg_count,
            "host_get_arg" => f_get_arg,
            "host_set_return" => f_set_return,
            "host_blake3" => f_blake3,
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_data_creation() {
        let data = HostData {
            state: BTreeMap::new(),
            dirty_keys: HashSet::new(),
            events: Vec::new(),
            transfers: Vec::new(),
            caller: "LOSWtestCaller".to_string(),
            self_address: "LOSConTestAddr".to_string(),
            balance: 1_000_000,
            timestamp: 1700000000,
            args: vec!["arg0".to_string(), "arg1".to_string()],
            return_data: Vec::new(),
            logs: Vec::new(),
            aborted: false,
            abort_message: String::new(),
        };
        assert_eq!(data.args.len(), 2);
        assert_eq!(data.balance, 1_000_000);
        assert!(!data.aborted);
    }

    #[test]
    fn test_host_state_is_send() {
        // Compile-time check: HostState must be Send + 'static for wasmer FunctionEnv
        fn assert_send<T: Send + 'static>() {}
        assert_send::<HostState>();
    }

    #[test]
    fn test_host_exec_result_defaults() {
        let result = HostExecResult {
            return_code: 0,
            return_data: Vec::new(),
            gas_used: 100,
            state_changes: BTreeMap::new(),
            events: Vec::new(),
            transfers: Vec::new(),
            logs: Vec::new(),
            aborted: false,
            abort_message: String::new(),
            sdk_mode: true,
        };
        assert_eq!(result.return_code, 0);
        assert!(result.sdk_mode);
        assert!(!result.aborted);
    }

    #[test]
    fn test_u128_reconstruction_from_i64_halves() {
        // Test the same bit manipulation used in host_transfer
        let amount: u128 = 1_000_000_000_000; // 1 trillion CIL
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;

        // Reconstruct
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, amount);

        // Test with a value that spans both halves
        let large: u128 = (42u128 << 64) | 0xDEAD_BEEF_CAFE_BABEu128;
        let lo2 = (large & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi2 = (large >> 64) as u64;
        let recon2 = ((hi2 as u128) << 64) | (lo2 as u128);
        assert_eq!(recon2, large);
    }

    #[test]
    fn test_limits_constants() {
        assert_eq!(MAX_STATE_VALUE_SIZE, 262_144); // 256 KB
        assert_eq!(MAX_STATE_KEY_SIZE, 1_024);
        assert_eq!(MAX_RETURN_DATA_SIZE, 262_144);
        assert_eq!(MAX_LOG_SIZE, 4_096);
        assert_eq!(MAX_EVENTS, 256);
        assert_eq!(MAX_TRANSFERS, 64);
        assert_eq!(MAX_STATE_KEYS, 1_024);
        assert_eq!(MAX_LOGS, 256);
    }
}
