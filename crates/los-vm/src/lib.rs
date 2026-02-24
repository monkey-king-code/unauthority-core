// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - VIRTUAL MACHINE (UVM)
//
// WASM-based smart contract execution engine.
// - Wasmer runtime with Cranelift compiler
// - Gas metering via Metering middleware
// - Sandboxed execution with resource limits
// - Host functions for state access, transfers, and events
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use wasmer::{imports, CompilerConfig, FunctionEnv, Instance, Module, Store, Value};
use wasmer_compiler_cranelift::Cranelift;
use wasmer_middlewares::metering::get_remaining_points;
use wasmer_middlewares::metering::MeteringPoints;
use wasmer_middlewares::Metering;

/// Global counter for leaked WASM timeout threads.
/// Once MAX_LEAKED_THREADS is reached, new WASM executions are rejected
/// to prevent unbounded resource consumption from pathological contracts.
static LEAKED_THREADS: AtomicUsize = AtomicUsize::new(0);
const MAX_LEAKED_THREADS: usize = 16;

// Provide __rust_probestack stub for wasmer-vm 4.x compatibility with
// Rust 1.85+ where this symbol was removed from compiler_builtins.
// Safe: the kernel provides guard pages for stack overflow on modern systems.
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos")
))]
#[no_mangle]
pub extern "C" fn __rust_probestack() {}

// Oracle module for exchange price feeds
pub mod oracle_connector;
// Host functions: bridge between WASM guest and LOS runtime
pub mod host;
// USP-01: Unauthority Standard for Permissionless Tokens
pub mod usp01;
// Token Registry: node-level USP-01 discovery and query helpers
pub mod token_registry;
// DEX Registry: node-level DEX pool discovery and query helpers
pub mod dex_registry;

/// Unauthority Virtual Machine (UVM)
/// Executes WebAssembly smart contracts with permissionless deployment
///
/// Maximum allowed WASM bytecode size (1 MB)
const MAX_BYTECODE_SIZE: usize = 1_048_576;
/// Maximum WASM execution time before timeout (5 seconds)
const MAX_EXECUTION_SECS: u64 = 5;
/// Gas cost per kilobyte of bytecode (compilation cost)
const GAS_PER_KB_BYTECODE: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub address: String,
    pub code_hash: String,
    pub bytecode: Vec<u8>,
    /// MAINNET: BTreeMap for deterministic contract state serialization
    pub state: BTreeMap<String, String>,
    pub balance: u128,
    pub created_at_block: u64,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCall {
    pub contract: String,
    pub function: String,
    pub args: Vec<String>,
    pub gas_limit: u64,
    /// Caller's LOS address (injected by node, verified via block signature).
    /// Empty string if not set (testnet mock dispatch only).
    #[serde(default)]
    pub caller: String,
    /// Block timestamp (seconds since epoch) for deterministic execution.
    /// All validators MUST use the SAME timestamp (from the block being processed)
    /// to ensure identical WASM execution results across the network.
    /// If 0, falls back to SystemTime::now() (backward-compatible, but non-deterministic).
    #[serde(default)]
    pub block_timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractResult {
    pub success: bool,
    pub output: String,
    pub gas_used: u64,
    pub state_changes: BTreeMap<String, String>,
    /// Events emitted by the contract during execution
    #[serde(default)]
    pub events: Vec<ContractEvent>,
    /// Transfers initiated by `host_transfer()` during execution.
    /// Each entry is (recipient_address, amount_cil).
    /// The contract's balance is already decremented — the caller MUST
    /// credit these amounts to the recipient accounts in the ledger.
    #[serde(default)]
    pub transfers: Vec<(String, u128)>,
}

/// Contract event (emitted during execution, stored for indexing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvent {
    pub contract: String,
    pub event_type: String,
    pub data: BTreeMap<String, String>,
    pub timestamp: u64,
}

/// WASM execution environment
pub struct WasmEngine {
    contracts: Arc<Mutex<BTreeMap<String, Contract>>>,
    nonce: Arc<Mutex<BTreeMap<String, u64>>>,
    /// Per-contract execution locks (TOCTOU prevention).
    /// Without this, two concurrent calls to the same contract would both
    /// snapshot the same state, execute independently, and overwrite each
    /// other's results. The lock ensures serialized execution per contract.
    contract_locks: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

impl WasmEngine {
    /// Create new WASM execution engine
    pub fn new() -> Self {
        WasmEngine {
            contracts: Arc::new(Mutex::new(BTreeMap::new())),
            nonce: Arc::new(Mutex::new(BTreeMap::new())),
            contract_locks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Deploy a WASM contract (Permissionless)
    pub fn deploy_contract(
        &self,
        owner: String,
        bytecode: Vec<u8>,
        initial_state: BTreeMap<String, String>,
        block_number: u64,
    ) -> Result<String, String> {
        // Validate WASM magic bytes (0x00 0x61 0x73 0x6d)
        if bytecode.len() < 4 || &bytecode[0..4] != b"\0asm" {
            return Err("Invalid WASM bytecode (missing magic header)".to_string());
        }

        // Enforce bytecode size limit
        if bytecode.len() > MAX_BYTECODE_SIZE {
            return Err(format!(
                "WASM bytecode too large: {} bytes (max {} bytes)",
                bytecode.len(),
                MAX_BYTECODE_SIZE
            ));
        }

        let mut nonce = self
            .nonce
            .lock()
            .map_err(|_| "Failed to lock nonce".to_string())?;

        let owner_nonce = nonce.entry(owner.clone()).or_insert(0);
        let contract_nonce = *owner_nonce;
        *owner_nonce = owner_nonce.saturating_add(1);

        // Deterministic contract address via blake3(owner || nonce || block)
        // Format: "LOSCon" + first 32 hex chars of blake3 hash
        let addr_input = format!("{}:{}:{}", owner, contract_nonce, block_number);
        let addr_hash = blake3::hash(addr_input.as_bytes());
        let address = format!("LOSCon{}", hex::encode(&addr_hash.as_bytes()[0..16]));

        // Calculate code hash
        let code_hash = hex::encode(&blake3::hash(&bytecode).as_bytes()[0..32]);

        let contract = Contract {
            address: address.clone(),
            code_hash,
            bytecode,
            state: initial_state,
            balance: 0,
            created_at_block: block_number,
            owner,
        };

        let mut contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        contracts.insert(address.clone(), contract);
        Ok(address)
    }

    /// Get contract by address
    pub fn get_contract(&self, address: &str) -> Result<Contract, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        contracts
            .get(address)
            .cloned()
            .ok_or_else(|| "Contract not found".to_string())
    }

    /// Execute real WASM bytecode using wasmer with deterministic instruction-level
    /// gas metering (wasmer-middlewares Metering) and a wall-clock timeout safety net.
    ///
    /// Gas metering is DETERMINISTIC: every WASM instruction costs exactly 1 gas unit.
    /// This ensures all validators compute identical gas usage for the same contract call,
    /// which is essential for consensus. The wall-clock timeout (MAX_EXECUTION_SECS) is
    /// kept as a safety net against pathological cases where metering overhead itself
    /// could stall the node.
    fn execute_wasm(
        &self,
        bytecode: &[u8],
        function: &str,
        args: &[i32],
        gas_limit: u64,
    ) -> Result<(i32, u64), String> {
        // W-07: Reject new WASM execution if too many threads are leaked from timeouts
        let leaked = LEAKED_THREADS.load(AtomicOrdering::Relaxed);
        if leaked >= MAX_LEAKED_THREADS {
            return Err(format!(
                "WASM execution rejected: {} leaked timeout threads (max {}). Node restart required.",
                leaked, MAX_LEAKED_THREADS
            ));
        }

        // 1. Bytecode size limit
        if bytecode.len() > MAX_BYTECODE_SIZE {
            return Err(format!(
                "WASM bytecode too large: {} bytes (max {} bytes)",
                bytecode.len(),
                MAX_BYTECODE_SIZE
            ));
        }

        // 2. Pre-calculate compilation gas cost
        let compile_gas = (bytecode.len() as u64 / 1024 + 1) * GAS_PER_KB_BYTECODE;
        if compile_gas > gas_limit {
            return Err(format!(
                "Out of gas: bytecode compilation cost {} exceeds gas limit {}",
                compile_gas, gas_limit
            ));
        }

        let remaining_gas = gas_limit - compile_gas;

        // 3. Clone data for thread-safe execution
        let bytecode_owned = bytecode.to_vec();
        let function_owned = function.to_string();
        let args_owned = args.to_vec();
        let abort_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let abort_clone = Arc::clone(&abort_flag);

        // 4. Execute in a separate thread with timeout
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        let _handle = std::thread::spawn(move || {
            // Check abort flag before each expensive phase
            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // DETERMINISTIC GAS METERING: Each WASM instruction costs 1 gas unit.
            // This is injected at compilation time by wasmer-middlewares::Metering.
            let cost_fn = |_operator: &wasmer::wasmparser::Operator| -> u64 { 1 };
            let metering = Arc::new(Metering::new(remaining_gas, cost_fn));

            let mut compiler = Cranelift::default();
            compiler.push_middleware(metering);
            let mut store = Store::new(compiler);

            let module = match Module::new(&store, &bytecode_owned) {
                Ok(m) => m,
                Err(e) => {
                    let _ = result_tx.send(Err(format!("Failed to compile WASM: {}", e)));
                    return;
                }
            };

            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            let import_object = imports! {};
            let instance = match Instance::new(&mut store, &module, &import_object) {
                Ok(i) => i,
                Err(e) => {
                    let _ = result_tx.send(Err(format!("Failed to instantiate WASM: {}", e)));
                    return;
                }
            };

            let func = match instance.exports.get_function(&function_owned) {
                Ok(f) => f,
                Err(e) => {
                    let _ = result_tx.send(Err(format!(
                        "Function '{}' not found: {}",
                        function_owned, e
                    )));
                    return;
                }
            };

            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            let wasm_args: Vec<Value> = args_owned.iter().map(|&v| Value::I32(v)).collect();

            let call_result = func.call(&mut store, &wasm_args);

            // If aborted during execution, don't send results
            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // Read remaining gas points from metering middleware
            let exec_gas = match get_remaining_points(&mut store, &instance) {
                MeteringPoints::Remaining(remaining) => remaining_gas - remaining,
                MeteringPoints::Exhausted => {
                    let _ = result_tx.send(Err(format!(
                        "Out of gas: execution exceeded {} instruction limit",
                        remaining_gas
                    )));
                    return;
                }
            };

            match call_result {
                Ok(results) => {
                    if let Some(Value::I32(val)) = results.first() {
                        let _ = result_tx.send(Ok((*val, exec_gas)));
                    } else {
                        let _ =
                            result_tx.send(Err("No return value from WASM function".to_string()));
                    }
                }
                Err(e) => {
                    // Check if the error is an out-of-gas trap from metering
                    let err_str = format!("{}", e);
                    if err_str.contains("unreachable") {
                        // Metering exhaustion triggers a trap
                        let _ = result_tx.send(Err(format!(
                            "Out of gas: execution exhausted {} gas limit",
                            remaining_gas
                        )));
                    } else {
                        let _ = result_tx.send(Err(format!("WASM execution failed: {}", e)));
                    }
                }
            }
        });

        // 5. Wait with timeout (safety net — deterministic metering should terminate first)
        let timeout = std::time::Duration::from_secs(MAX_EXECUTION_SECS);
        match result_rx.recv_timeout(timeout) {
            Ok(Ok((value, exec_gas))) => {
                let total_gas = compile_gas + exec_gas;
                if total_gas > gas_limit {
                    return Err(format!(
                        "Out of gas: used {} (compile: {} + exec: {}) > limit {}",
                        total_gas, compile_gas, exec_gas, gas_limit
                    ));
                }
                Ok((value, total_gas))
            }
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // SECURITY: Set abort flag so the thread exits at its next checkpoint
                abort_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                // W-07: Track leaked thread count
                LEAKED_THREADS.fetch_add(1, AtomicOrdering::Relaxed);
                // Do NOT join — if WASM entered an infinite loop inside
                // func.call(), the thread is permanently stuck and join() would block
                // the calling thread forever. Let the thread leak (bounded damage).
                Err(format!(
                    "WASM execution timeout: exceeded {} second limit",
                    MAX_EXECUTION_SECS
                ))
            }
            Err(e) => Err(format!("WASM execution channel error: {}", e)),
        }
    }

    /// Execute WASM bytecode with full host function support (SDK mode + legacy fallback).
    ///
    /// Host functions allow contracts to read/write state, emit events, transfer CIL,
    /// and access caller context — all via WASM imports (module "env").
    ///
    /// **Calling convention:**
    /// - SDK contracts: exported function takes no WASM params, returns `i32` status code.
    ///   Args are read via `host_get_arg()`, return data via `host_set_return()`.
    /// - Legacy contracts: exported function takes `i32` params directly, returns `i32` result.
    ///   Detected automatically by checking the function's WASM type signature.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_wasm_hosted(
        &self,
        bytecode: &[u8],
        function: &str,
        args: &[String],
        gas_limit: u64,
        caller: &str,
        contract_addr: &str,
        contract_state: &BTreeMap<String, String>,
        balance: u128,
        timestamp: u64,
    ) -> Result<host::HostExecResult, String> {
        use host::{HostData, HostExecResult, HostState};
        use std::collections::HashSet;

        // Reuse the same safety checks as execute_wasm
        let leaked = LEAKED_THREADS.load(AtomicOrdering::Relaxed);
        if leaked >= MAX_LEAKED_THREADS {
            return Err(format!(
                "WASM execution rejected: {} leaked timeout threads (max {}). Node restart required.",
                leaked, MAX_LEAKED_THREADS
            ));
        }
        if bytecode.len() > MAX_BYTECODE_SIZE {
            return Err(format!(
                "WASM bytecode too large: {} bytes (max {} bytes)",
                bytecode.len(),
                MAX_BYTECODE_SIZE
            ));
        }
        let compile_gas = (bytecode.len() as u64 / 1024 + 1) * GAS_PER_KB_BYTECODE;
        if compile_gas > gas_limit {
            return Err(format!(
                "Out of gas: bytecode compilation cost {} exceeds gas limit {}",
                compile_gas, gas_limit
            ));
        }
        let remaining_gas = gas_limit - compile_gas;

        // Convert contract state (String→String) to byte state (String→Vec<u8>)
        let state_bytes: BTreeMap<String, Vec<u8>> = contract_state
            .iter()
            .map(|(k, v)| (k.clone(), v.as_bytes().to_vec()))
            .collect();

        // Shared host data (accessed by host functions inside the WASM thread,
        // then read back by the caller after execution completes).
        let host_data = Arc::new(Mutex::new(HostData {
            state: state_bytes,
            dirty_keys: HashSet::new(),
            events: Vec::new(),
            transfers: Vec::new(),
            caller: caller.to_string(),
            self_address: contract_addr.to_string(),
            balance,
            timestamp,
            args: args.to_vec(),
            return_data: Vec::new(),
            logs: Vec::new(),
            aborted: false,
            abort_message: String::new(),
        }));
        let host_data_thread = Arc::clone(&host_data);

        let bytecode_owned = bytecode.to_vec();
        let function_owned = function.to_string();
        let args_owned = args.to_vec();
        let abort_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let abort_clone = Arc::clone(&abort_flag);

        let (result_tx, result_rx) = std::sync::mpsc::channel::<Result<(i32, u64, bool), String>>();

        let _handle = std::thread::spawn(move || {
            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // Deterministic gas metering: 1 WASM instruction = 1 gas unit
            let cost_fn = |_operator: &wasmer::wasmparser::Operator| -> u64 { 1 };
            let metering = Arc::new(wasmer_middlewares::Metering::new(remaining_gas, cost_fn));

            let mut compiler = Cranelift::default();
            compiler.push_middleware(metering);
            let mut store = Store::new(compiler);

            let module = match Module::new(&store, &bytecode_owned) {
                Ok(m) => m,
                Err(e) => {
                    let _ = result_tx.send(Err(format!("Failed to compile WASM: {}", e)));
                    return;
                }
            };

            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // Create FunctionEnv with host state (memory set after instantiation)
            let host_state = HostState {
                memory: None,
                inner: host_data_thread,
            };
            let env = FunctionEnv::new(&mut store, host_state);

            // Create imports with all 16 host functions
            let import_object = host::create_host_imports(&mut store, &env);

            let instance = match Instance::new(&mut store, &module, &import_object) {
                Ok(i) => i,
                Err(first_err) => {
                    // Module may not import "env" at all — retry with empty imports.
                    // WARNING: This means NO host functions (transfer, log, storage, etc.)
                    // are available. Only pure-compute WASM modules should reach this path.
                    eprintln!(
                        "⚠️ VM: WASM module instantiation with host imports failed ({}). \
                         Retrying with empty imports (no host functions available).",
                        first_err
                    );
                    match Instance::new(&mut store, &module, &imports! {}) {
                        Ok(i) => i,
                        Err(e) => {
                            let _ =
                                result_tx.send(Err(format!("Failed to instantiate WASM: {}", e)));
                            return;
                        }
                    }
                }
            };

            // Set memory reference in env (so host functions can read/write guest memory)
            if let Ok(memory) = instance.exports.get_memory("memory") {
                env.as_mut(&mut store).memory = Some(memory.clone());
            }

            let func = match instance.exports.get_function(&function_owned) {
                Ok(f) => f,
                Err(e) => {
                    let _ = result_tx.send(Err(format!(
                        "Function '{}' not found: {}",
                        function_owned, e
                    )));
                    return;
                }
            };

            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // Auto-detect calling convention from function signature
            let func_type = func.ty(&store);
            let params = func_type.params();
            let is_sdk_mode = params.is_empty();

            let call_result = if is_sdk_mode {
                // SDK mode: no WASM-level args; contract reads via host_get_arg()
                func.call(&mut store, &[])
            } else {
                // Legacy mode: convert string args to i32 values
                let mut wasm_args: Vec<Value> = args_owned
                    .iter()
                    .map(|s| Value::I32(s.parse::<i32>().unwrap_or(0)))
                    .collect();
                // Pad with zeros if fewer args than params, truncate if more
                while wasm_args.len() < params.len() {
                    wasm_args.push(Value::I32(0));
                }
                wasm_args.truncate(params.len());
                func.call(&mut store, &wasm_args)
            };

            if abort_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            // Read remaining gas
            let exec_gas =
                match wasmer_middlewares::metering::get_remaining_points(&mut store, &instance) {
                    wasmer_middlewares::metering::MeteringPoints::Remaining(r) => remaining_gas - r,
                    wasmer_middlewares::metering::MeteringPoints::Exhausted => {
                        let _ = result_tx.send(Err(format!(
                            "Out of gas: execution exceeded {} instruction limit",
                            remaining_gas
                        )));
                        return;
                    }
                };

            match call_result {
                Ok(results) => {
                    let return_code = results
                        .first()
                        .and_then(|v| {
                            if let Value::I32(x) = v {
                                Some(*x)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    let _ = result_tx.send(Ok((return_code, exec_gas, is_sdk_mode)));
                }
                Err(e) => {
                    let err_str = format!("{}", e);
                    if err_str.contains("unreachable") {
                        // Could be metering exhaustion OR contract abort
                        let _ = result_tx
                            .send(Err(format!("WASM trap (abort or out of gas): {}", err_str)));
                    } else {
                        let _ = result_tx.send(Err(format!("WASM execution failed: {}", e)));
                    }
                }
            }
        });

        // Wait with timeout (safety net)
        let timeout = std::time::Duration::from_secs(MAX_EXECUTION_SECS);
        match result_rx.recv_timeout(timeout) {
            Ok(Ok((return_code, exec_gas, is_sdk_mode))) => {
                let total_gas = compile_gas + exec_gas;
                if total_gas > gas_limit {
                    return Err(format!(
                        "Out of gas: used {} (compile: {} + exec: {}) > limit {}",
                        total_gas, compile_gas, exec_gas, gas_limit
                    ));
                }

                // Extract results from shared host data
                let data = host_data
                    .lock()
                    .map_err(|_| "Failed to lock host data".to_string())?;

                if data.aborted {
                    return Err(format!("Contract aborted: {}", data.abort_message));
                }

                // Extract only dirty (modified) keys as state changes
                let state_changes: BTreeMap<String, Vec<u8>> = data
                    .dirty_keys
                    .iter()
                    .filter_map(|k| data.state.get(k).map(|v| (k.clone(), v.clone())))
                    .collect();

                Ok(HostExecResult {
                    return_code,
                    return_data: data.return_data.clone(),
                    gas_used: total_gas,
                    state_changes,
                    events: data.events.clone(),
                    transfers: data.transfers.clone(),
                    logs: data.logs.clone(),
                    aborted: false,
                    abort_message: String::new(),
                    sdk_mode: is_sdk_mode,
                })
            }
            Ok(Err(e)) => {
                // Check if abort was set before the error
                if let Ok(d) = host_data.lock() {
                    if d.aborted {
                        return Err(format!("Contract aborted: {}", d.abort_message));
                    }
                }
                Err(e)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                abort_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                LEAKED_THREADS.fetch_add(1, AtomicOrdering::Relaxed);
                Err(format!(
                    "WASM execution timeout: exceeded {} second limit",
                    MAX_EXECUTION_SECS
                ))
            }
            Err(e) => Err(format!("WASM execution channel error: {}", e)),
        }
    }

    /// Get or create a per-contract execution lock (C-07 TOCTOU fix).
    fn get_contract_lock(&self, contract_addr: &str) -> Arc<Mutex<()>> {
        let mut locks = self
            .contract_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        locks
            .entry(contract_addr.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Try hosted WASM execution for a contract call.
    /// Returns `Ok(Some(result))` on success, `Ok(None)` if fallback is needed,
    /// or `Err(e)` for fatal errors that should propagate immediately.
    ///
    /// Acquires a per-contract lock to prevent TOCTOU races.
    /// Without this, two concurrent calls to the same contract would snapshot the
    /// same state, execute independently, and the second write would silently
    /// overwrite the first's state changes.
    fn try_hosted_call(&self, call: &ContractCall) -> Result<Option<ContractResult>, String> {
        // C-07: Acquire per-contract execution lock (serializes concurrent calls)
        let contract_lock = self.get_contract_lock(&call.contract);
        let _guard = contract_lock
            .lock()
            .map_err(|_| "Failed to acquire contract execution lock".to_string())?;

        // Get contract snapshot (short lock, released before execution)
        let contract_snapshot = {
            let contracts = self
                .contracts
                .lock()
                .map_err(|_| "Failed to lock contracts".to_string())?;
            match contracts.get(&call.contract) {
                Some(c) => c.clone(),
                None => return Ok(None), // Let main code handle "not found"
            }
        }; // lock released

        // Must be valid WASM to attempt hosted execution
        if contract_snapshot.bytecode.len() < 4 || !contract_snapshot.bytecode.starts_with(b"\0asm")
        {
            return Ok(None);
        }

        // DETERMINISM FIX: Use block timestamp for reproducible execution.
        // All validators processing the same block MUST get identical results.
        // Fallback to SystemTime::now() only if block_timestamp is 0 (legacy calls).
        let timestamp = if call.block_timestamp > 0 {
            call.block_timestamp
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        };

        match self.execute_wasm_hosted(
            &contract_snapshot.bytecode,
            &call.function,
            &call.args,
            call.gas_limit,
            &call.caller,
            &call.contract,
            &contract_snapshot.state,
            contract_snapshot.balance,
            timestamp,
        ) {
            Ok(exec_result) => {
                // Apply state changes + transfers back to contract (short lock)
                if !exec_result.state_changes.is_empty() || !exec_result.transfers.is_empty() {
                    let mut contracts = self
                        .contracts
                        .lock()
                        .map_err(|_| "Failed to lock contracts for state update".to_string())?;
                    if let Some(c) = contracts.get_mut(&call.contract) {
                        for (key, val) in &exec_result.state_changes {
                            c.state
                                .insert(key.clone(), String::from_utf8_lossy(val).to_string());
                        }
                        for (_, amount) in &exec_result.transfers {
                            c.balance = c.balance.saturating_sub(*amount);
                        }
                    }
                }

                let (success, output) = if exec_result.sdk_mode {
                    (
                        exec_result.return_code == 0,
                        if exec_result.return_data.is_empty() {
                            exec_result.return_code.to_string()
                        } else {
                            String::from_utf8_lossy(&exec_result.return_data).to_string()
                        },
                    )
                } else {
                    // Legacy: return_code IS the result, always success
                    (true, exec_result.return_code.to_string())
                };

                Ok(Some(ContractResult {
                    success,
                    output,
                    gas_used: exec_result.gas_used,
                    state_changes: exec_result
                        .state_changes
                        .iter()
                        .map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).to_string()))
                        .collect(),
                    events: exec_result.events,
                    transfers: exec_result.transfers,
                }))
            }
            Err(e)
                if e.contains("Out of gas")
                    || e.contains("timeout")
                    || e.contains("too large")
                    || e.contains("leaked")
                    || e.contains("aborted") =>
            {
                Err(e) // Fatal — propagate
            }
            Err(_) => Ok(None), // Non-fatal — fall through to legacy/mock
        }
    }

    /// Execute contract function.
    ///
    /// Execution order:
    /// 1. **Hosted WASM** (SDK mode with host functions) — preferred path
    /// 2. **Legacy WASM** (i32 args, no host functions) — backward compatibility
    /// 3. **Mock dispatch** (testnet only) — disabled on mainnet
    pub fn call_contract(&self, call: ContractCall) -> Result<ContractResult, String> {
        // ── Phase 1: Try hosted WASM execution (SDK + legacy auto-detect) ──
        if let Some(result) = self.try_hosted_call(&call)? {
            return Ok(result);
        }

        // ── Phase 2: Legacy WASM execution (backward compat, i32 args only) ──
        {
            let contracts = self
                .contracts
                .lock()
                .map_err(|_| "Failed to lock contracts".to_string())?;
            let contract = match contracts.get(&call.contract) {
                Some(c) => c,
                None => return Err("Contract not found".to_string()),
            };

            if contract.bytecode.len() >= 8 {
                if let Ok(i32_args) = call
                    .args
                    .iter()
                    .map(|s| s.parse::<i32>())
                    .collect::<Result<Vec<_>, _>>()
                {
                    match self.execute_wasm(
                        &contract.bytecode,
                        &call.function,
                        &i32_args,
                        call.gas_limit,
                    ) {
                        Ok((result, gas_used)) => {
                            return Ok(ContractResult {
                                success: true,
                                output: result.to_string(),
                                gas_used,
                                state_changes: BTreeMap::new(),
                                events: Vec::new(),
                                transfers: Vec::new(),
                            });
                        }
                        Err(e)
                            if e.contains("Out of gas")
                                || e.contains("timeout")
                                || e.contains("too large") =>
                        {
                            return Err(e);
                        }
                        Err(_) => {
                            // Fall through to mock dispatch
                        }
                    }
                }
            }
        }

        // ── Phase 3: Mock dispatch (testnet only) ──
        // SECURITY: Mock dispatch is DISABLED on mainnet builds.
        #[cfg(feature = "mainnet")]
        return Err(format!(
            "Contract function '{}' not found in WASM module. Mock dispatch disabled on mainnet.",
            call.function
        ));

        // Fallback to mock dispatch for testing/simple contracts (testnet only)
        #[cfg(not(feature = "mainnet"))]
        {
            let mut contracts = self
                .contracts
                .lock()
                .map_err(|_| "Failed to lock contracts for mock dispatch".to_string())?;
            let contract = contracts
                .get_mut(&call.contract)
                .ok_or("Contract not found".to_string())?;

            let (output, gas_used, state_changes) = match call.function.as_str() {
                "transfer" => {
                    if call.args.len() < 2 {
                        return Err("transfer requires: amount, recipient".to_string());
                    }
                    let amount: u128 = call.args[0]
                        .parse()
                        .map_err(|_| "Invalid amount".to_string())?;

                    if contract.balance < amount {
                        return Err("Insufficient contract balance".to_string());
                    }

                    contract.balance -= amount;
                    (format!("Transferred {} cil", amount), 75, BTreeMap::new())
                }
                "mint" => {
                    // SECURITY P1-3: Minting via contract is DISABLED
                    // Only the blockchain consensus (VOTE_RES flow) may mint LOS.
                    // Allowing contracts to mint would bypass supply controls.
                    return Err(
                        "mint: operation not permitted — LOS minting requires PoW consensus"
                            .to_string(),
                    );
                }
                "burn" => {
                    if call.args.is_empty() {
                        return Err("burn requires: amount".to_string());
                    }
                    let amount: u128 = call.args[0]
                        .parse()
                        .map_err(|_| "Invalid amount".to_string())?;

                    if contract.balance < amount {
                        return Err("Insufficient balance to burn".to_string());
                    }

                    contract.balance -= amount;
                    (format!("Burned {} cil", amount), 100, BTreeMap::new())
                }
                "set_state" => {
                    if call.args.len() < 2 {
                        return Err("set_state requires: key, value".to_string());
                    }
                    let key = call.args[0].clone();
                    let value = call.args[1].clone();

                    let mut sc: BTreeMap<String, String> = BTreeMap::new();
                    sc.insert(key, value);
                    ("State updated".to_string(), 60, sc)
                }
                "get_state" => {
                    if call.args.is_empty() {
                        return Err("get_state requires: key".to_string());
                    }
                    let key = &call.args[0];
                    let value = contract
                        .state
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| "null".to_string());

                    (value, 30, BTreeMap::new())
                }
                "get_balance" => (format!("{}", contract.balance), 20, BTreeMap::new()),
                _ => {
                    return Err(format!("Unknown function: {}", call.function));
                }
            };

            // Check gas limit
            if gas_used > call.gas_limit {
                return Err(format!("Out of gas: {} > {}", gas_used, call.gas_limit));
            }

            // Apply state changes
            for (k, v) in state_changes.iter() {
                contract.state.insert(k.clone(), v.clone());
            }

            Ok(ContractResult {
                success: true,
                output,
                gas_used,
                state_changes,
                events: Vec::new(),
                transfers: Vec::new(),
            })
        } // end #[cfg(not(feature = "mainnet"))]
    }

    /// Send native cil to contract
    pub fn send_to_contract(&self, contract_addr: &str, amount: u128) -> Result<(), String> {
        let mut contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        let contract = contracts
            .get_mut(contract_addr)
            .ok_or("Contract not found")?;

        contract.balance = contract.balance.saturating_add(amount);
        Ok(())
    }

    /// Check if contract exists
    pub fn contract_exists(&self, address: &str) -> Result<bool, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        Ok(contracts.contains_key(address))
    }

    /// List all deployed contracts
    pub fn list_contracts(&self) -> Result<Vec<String>, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        Ok(contracts.keys().cloned().collect())
    }

    /// Get contract count
    pub fn contract_count(&self) -> Result<usize, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        Ok(contracts.len())
    }

    /// Get contract state
    pub fn get_contract_state(&self, address: &str) -> Result<BTreeMap<String, String>, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;

        let contract = contracts.get(address).ok_or("Contract not found")?;

        Ok(contract.state.clone())
    }
}

impl Default for WasmEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────
// PERSISTENCE: Serialize/Deserialize all contract state
// ─────────────────────────────────────────────────────────────────

impl WasmEngine {
    /// Serialize all contracts + nonce state for persistence (sled DB).
    /// Bytecode is included so peers can re-load without re-fetching.
    pub fn serialize_all(&self) -> Result<Vec<u8>, String> {
        let contracts = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;
        let nonce = self
            .nonce
            .lock()
            .map_err(|_| "Failed to lock nonce".to_string())?;
        let data = serde_json::json!({
            "contracts": &*contracts,
            "nonce": &*nonce,
        });
        serde_json::to_vec(&data).map_err(|e| format!("Failed to serialize VM state: {}", e))
    }

    /// Deserialize and restore all contracts + nonce state from persistence.
    pub fn deserialize_all(&self, data: &[u8]) -> Result<usize, String> {
        #[derive(Deserialize)]
        struct VmSnapshot {
            contracts: BTreeMap<String, Contract>,
            nonce: BTreeMap<String, u64>,
        }
        let snapshot: VmSnapshot = serde_json::from_slice(data)
            .map_err(|e| format!("Failed to deserialize VM state: {}", e))?;

        let count = snapshot.contracts.len();

        let mut c = self
            .contracts
            .lock()
            .map_err(|_| "Failed to lock contracts".to_string())?;
        *c = snapshot.contracts;

        let mut n = self
            .nonce
            .lock()
            .map_err(|_| "Failed to lock nonce".to_string())?;
        *n = snapshot.nonce;

        Ok(count)
    }

    /// Get the blake3 code hash for given bytecode (used for DEPLOY link verification)
    pub fn compute_code_hash(bytecode: &[u8]) -> String {
        hex::encode(&blake3::hash(bytecode).as_bytes()[0..32])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_engine_creation() {
        let engine = WasmEngine::new();
        assert_eq!(engine.contract_count().unwrap(), 0);
        assert!(engine.list_contracts().unwrap().is_empty());
    }

    #[test]
    fn test_deploy_contract() {
        let engine = WasmEngine::new();

        // Create minimal WASM bytecode (magic header only)
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();
        let owner = "alice".to_string();

        let result = engine.deploy_contract(owner, wasm_bytes, BTreeMap::new(), 1);
        assert!(result.is_ok());

        let addr = result.unwrap();
        assert!(addr.starts_with("LOSCon"));
        assert_eq!(engine.contract_count().unwrap(), 1);
    }

    #[test]
    fn test_invalid_wasm_bytecode() {
        let engine = WasmEngine::new();
        let invalid_bytes = vec![0x00, 0x00, 0x00, 0x00];

        let result = engine.deploy_contract("alice".to_string(), invalid_bytes, BTreeMap::new(), 1);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid WASM"));
    }

    #[test]
    fn test_get_contract() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();
        let owner = "bob".to_string();

        let addr = engine
            .deploy_contract(owner.clone(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();
        let contract = engine.get_contract(&addr).unwrap();

        assert_eq!(contract.owner, owner);
        assert_eq!(contract.balance, 0);
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_call_transfer() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("charlie".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        // Send balance to contract first
        engine.send_to_contract(&addr, 1000).unwrap();

        let call = ContractCall {
            contract: addr,
            function: "transfer".to_string(),
            args: vec!["500".to_string(), "recipient".to_string()],
            gas_limit: 1000,
            caller: "charlie".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call).unwrap();
        assert!(result.success);
        assert_eq!(result.gas_used, 75);
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_call_set_get_state() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("dave".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        // Set state
        let set_call = ContractCall {
            contract: addr.clone(),
            function: "set_state".to_string(),
            args: vec!["counter".to_string(), "42".to_string()],
            gas_limit: 1000,
            caller: "dave".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(set_call).unwrap();
        assert!(result.success);

        // Get state
        let get_call = ContractCall {
            contract: addr.clone(),
            function: "get_state".to_string(),
            args: vec!["counter".to_string()],
            gas_limit: 1000,
            caller: "dave".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(get_call).unwrap();
        assert_eq!(result.output, "42");
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_contract_balance() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("eve".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        engine.send_to_contract(&addr, 5000).unwrap();

        let call = ContractCall {
            contract: addr,
            function: "get_balance".to_string(),
            args: vec![],
            gas_limit: 100,
            caller: "eve".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call).unwrap();
        assert_eq!(result.output, "5000");
    }

    #[test]
    fn test_call_nonexistent_contract() {
        let engine = WasmEngine::new();
        let call = ContractCall {
            contract: "nonexistent".to_string(),
            function: "transfer".to_string(),
            args: vec![],
            gas_limit: 1000,
            caller: "nobody".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call);
        assert!(result.is_err());
    }

    #[test]
    fn test_send_to_contract() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("frank".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        let result = engine.send_to_contract(&addr, 2500);
        assert!(result.is_ok());

        let contract = engine.get_contract(&addr).unwrap();
        assert_eq!(contract.balance, 2500);
    }

    #[test]
    fn test_multiple_deployments_increment_nonce() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();
        let owner = "grace".to_string();

        let addr1 = engine
            .deploy_contract(owner.clone(), wasm_bytes.clone(), BTreeMap::new(), 1)
            .unwrap();
        let addr2 = engine
            .deploy_contract(owner, wasm_bytes, BTreeMap::new(), 2)
            .unwrap();

        assert_ne!(addr1, addr2);
        assert_eq!(engine.contract_count().unwrap(), 2);
    }

    #[test]
    fn test_contract_list() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        for i in 0..3 {
            let owner = format!("user_{}", i);
            let _ = engine.deploy_contract(owner, wasm_bytes.clone(), BTreeMap::new(), i);
        }

        let contracts = engine.list_contracts().unwrap();
        assert_eq!(contracts.len(), 3);
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_gas_limit_exceeded() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("henry".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        engine.send_to_contract(&addr, 1000).unwrap();

        // transfer costs 75 gas, so set limit to 50 to exceed
        let call = ContractCall {
            contract: addr,
            function: "transfer".to_string(),
            args: vec!["500".to_string(), "recipient".to_string()],
            gas_limit: 50, // Too low
            caller: "henry".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Out of gas"));
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_unknown_function() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("iris".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        let call = ContractCall {
            contract: addr,
            function: "unknown_func".to_string(),
            args: vec![],
            gas_limit: 1000,
            caller: "iris".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown function"));
    }

    #[test]
    fn test_contract_result_serialization() {
        let result = ContractResult {
            success: true,
            output: "success".to_string(),
            gas_used: 100,
            state_changes: BTreeMap::new(),
            events: Vec::new(),
            transfers: Vec::new(),
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ContractResult = serde_json::from_str(&json).unwrap();

        assert!(deserialized.success);
        assert_eq!(deserialized.output, "success");
    }

    #[test]
    #[cfg(not(feature = "mainnet"))]
    fn test_get_contract_state() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("jack".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        // Set some state
        let call = ContractCall {
            contract: addr.clone(),
            function: "set_state".to_string(),
            args: vec!["name".to_string(), "test".to_string()],
            gas_limit: 100,
            caller: "jack".to_string(),
            block_timestamp: 0,
        };

        engine.call_contract(call).unwrap();

        let state = engine.get_contract_state(&addr).unwrap();
        assert_eq!(state.get("name"), Some(&"test".to_string()));
    }

    #[test]
    fn test_contract_exists() {
        let engine = WasmEngine::new();
        let wasm_bytes = b"\0asm\x01\x00\x00\x00".to_vec();

        let addr = engine
            .deploy_contract("kate".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        assert!(engine.contract_exists(&addr).unwrap());
        assert!(!engine.contract_exists("nonexistent").unwrap());
    }

    #[test]
    fn test_real_wasm_execution() {
        let engine = WasmEngine::new();

        // Real WASM bytecode: (module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))
        let wasm_bytes = vec![
            0x00, 0x61, 0x73, 0x6d, // magic: \0asm
            0x01, 0x00, 0x00, 0x00, // version: 1
            0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01,
            0x7f, // type section: (i32,i32)->i32
            0x03, 0x02, 0x01, 0x00, // function section: func 0 uses type 0
            0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00,
            0x00, // export section: "add" = func 0
            0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a,
            0x0b, // code: local.get 0, local.get 1, i32.add
        ];

        let addr = engine
            .deploy_contract("wasm_test".to_string(), wasm_bytes, BTreeMap::new(), 1)
            .unwrap();

        let call = ContractCall {
            contract: addr,
            function: "add".to_string(),
            args: vec!["5".to_string(), "7".to_string()],
            gas_limit: 1000,
            caller: "wasm_tester".to_string(),
            block_timestamp: 0,
        };

        let result = engine.call_contract(call).unwrap();
        assert!(result.success);
        assert_eq!(result.output, "12"); // 5 + 7 = 12
    }
}
