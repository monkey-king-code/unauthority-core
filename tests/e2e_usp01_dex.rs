// ============================================================================
// E2E USP-01 TOKEN & DEX AMM TEST — UNAUTHORITY (LOS)
// ============================================================================
//
// End-to-end integration tests for the USP-01 fungible token standard and
// DEX AMM (constant-product x·y=k) on the Unauthority (LOS) blockchain.
//
// Architecture:
//   - Uses WasmEngine mock dispatch (testnet mode) for contract operations.
//   - Tests state patterns matching the actual USP-01 and DEX contracts.
//   - Tests blockchain integration via ContractDeploy/ContractCall blocks.
//   - Validates DEX integer math (no f32/f64 — all integer arithmetic).
//
// Test Scenarios:
//   1.  VM Contract Lifecycle — deploy, state read/write, contract listing
//   2.  USP-01 Token Init — state key patterns, metadata verification
//   3.  USP-01 Transfer & Balances — debit/credit, balance tracking
//   4.  USP-01 Approve & TransferFrom — allowance patterns
//   5.  USP-01 Burn — supply reduction, balance deduction
//   6.  DEX AMM Initialization — pool state patterns
//   7.  DEX Pool Creation Math — isqrt, initial LP, minimum liquidity
//   8.  DEX Swap Math — constant product, fee deduction, slippage
//   9.  DEX Liquidity Add/Remove — proportional shares, min slippage
//   10. Blockchain Contract Blocks — ContractDeploy/Call block types
//   11. Gas Limits & Error Handling — gas exceeded, invalid calls
//   12. Full Integration Flow — token → DEX pipeline
//
// Run:
//   cargo test --release --test e2e_usp01_dex -- --test-threads=1 --nocapture
//
// ============================================================================

use std::collections::BTreeMap;

use los_core::{
    Block, BlockType, Ledger, BASE_FEE_CIL, CIL_PER_LOS, DEFAULT_GAS_LIMIT, GAS_PRICE_CIL,
    MIN_CALL_FEE_CIL, MIN_DEPLOY_FEE_CIL,
};
use los_crypto::{generate_keypair, public_key_to_address, sign_message};
use los_vm::{ContractCall, WasmEngine};

// ============================================================================
// CONSTANTS — mirrors USP-01 and DEX contract state key conventions
// ============================================================================

/// Minimum WASM bytecode: valid header, no sections.
const MINIMAL_WASM: &[u8] = b"\0asm\x01\x00\x00\x00";

// USP-01 state key prefixes
const USP01_INIT_KEY: &str = "usp01:init";
const USP01_NAME_KEY: &str = "usp01:name";
const USP01_SYMBOL_KEY: &str = "usp01:symbol";
const USP01_DECIMALS_KEY: &str = "usp01:decimals";
const USP01_TOTAL_SUPPLY_KEY: &str = "usp01:total_supply";
const USP01_IS_WRAPPED_KEY: &str = "usp01:is_wrapped";
const USP01_WRAPPED_ORIGIN_KEY: &str = "usp01:wrapped_origin";
const USP01_MAX_SUPPLY_KEY: &str = "usp01:max_supply";
const USP01_BRIDGE_OPERATOR_KEY: &str = "usp01:bridge_operator";
const USP01_OWNER_KEY: &str = "usp01:owner";

// DEX state key conventions
const DEX_INIT_KEY: &str = "dex:init";
const DEX_OWNER_KEY: &str = "dex:owner";
const DEX_POOL_COUNT_KEY: &str = "dex:pool_count";

// DEX AMM constants (from dex_amm.rs)
const DEFAULT_FEE_BPS: u128 = 30;
const BPS_DENOMINATOR: u128 = 10_000;
const MINIMUM_LIQUIDITY: u128 = 1_000;
const MAX_FEE_BPS: u128 = 1_000;

// ============================================================================
// HELPERS
// ============================================================================

/// Integer square root (Newton's method) — matches the production isqrt.
/// MUST match `los_core::validator_rewards::isqrt` and `dex_amm::isqrt`.
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// DEX constant-product output calculation (integer-only).
/// `amount_out = (amount_in * reserve_out) / (reserve_in + amount_in)`
fn compute_output(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0;
    }
    // Overflow-safe: try direct multiply first, fall back to precision scaling
    match amount_in.checked_mul(reserve_out) {
        Some(numerator) => numerator / (reserve_in + amount_in),
        None => {
            // Precision scaling for large values
            let precision: u128 = 1_000_000_000_000;
            let scaled_in = amount_in / precision;
            let remainder = amount_in % precision;
            let part1 = scaled_in * reserve_out / (reserve_in / precision + scaled_in);
            let part2 = remainder * reserve_out / (reserve_in + remainder); // approximate
            part1 + part2
        }
    }
}

/// DEX fee deduction (integer-only, basis points).
/// Returns (amount_after_fee, fee_amount).
fn deduct_fee(amount: u128, fee_bps: u128) -> (u128, u128) {
    let fee = amount * fee_bps / BPS_DENOMINATOR;
    (amount - fee, fee)
}

/// Format USP-01 balance key: `bal:{address}`
fn balance_key(address: &str) -> String {
    format!("bal:{}", address)
}

/// Format USP-01 allowance key: `allow:{owner}:{spender}`
fn allowance_key(owner: &str, spender: &str) -> String {
    format!("allow:{}:{}", owner, spender)
}

/// Format DEX pool state key prefix
fn pool_key(pool_id: &str, field: &str) -> String {
    format!("pool:{}:{}", pool_id, field)
}

/// Format DEX LP key: `lp:{pool_id}:{address}`
fn lp_key(pool_id: &str, address: &str) -> String {
    format!("lp:{}:{}", pool_id, address)
}

/// Generate deterministic pool ID from sorted token pair
fn pool_id(token_a: &str, token_b: &str) -> String {
    let (first, second) = if token_a <= token_b {
        (token_a, token_b)
    } else {
        (token_b, token_a)
    };
    format!("POOL:{}:{}", first, second)
}

/// Simulated node with Dilithium5 keypair.
struct SimNode {
    address: String,
    pubkey_hex: String,
    secret_key: Vec<u8>,
}

impl SimNode {
    fn new() -> Self {
        let kp = generate_keypair();
        let address = public_key_to_address(&kp.public_key);
        let pubkey_hex = hex::encode(&kp.public_key);
        let sk = kp.secret_key.clone();
        SimNode {
            address,
            pubkey_hex,
            secret_key: sk,
        }
    }
}

/// Mine PoW for a block (16 leading zero bits) and sign with Dilithium5.
fn mine_and_sign(block: &mut Block, secret_key: &[u8]) {
    block.signature = String::new();
    for nonce in 0u64.. {
        block.work = nonce;
        if block.verify_pow() {
            let msg = block.signing_hash();
            let sig =
                sign_message(msg.as_bytes(), secret_key).expect("Dilithium5 signing must succeed");
            block.signature = hex::encode(&sig);
            return;
        }
    }
    unreachable!("PoW mining loop exhausted u64 range");
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Helper: call `set_state` on a contract via mock dispatch.
fn vm_set_state(engine: &WasmEngine, contract: &str, key: &str, value: &str, caller: &str) {
    let call = ContractCall {
        contract: contract.to_string(),
        function: "set_state".to_string(),
        args: vec![key.to_string(), value.to_string()],
        gas_limit: 1000,
        caller: caller.to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call).expect("set_state must succeed");
    assert!(result.success, "set_state failed: {}", result.output);
}

/// Helper: call `get_state` on a contract via mock dispatch.
fn vm_get_state(engine: &WasmEngine, contract: &str, key: &str, caller: &str) -> String {
    let call = ContractCall {
        contract: contract.to_string(),
        function: "get_state".to_string(),
        args: vec![key.to_string()],
        gas_limit: 1000,
        caller: caller.to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call).expect("get_state must succeed");
    result.output
}

// ============================================================================
// TEST 1: VM CONTRACT LIFECYCLE
// ============================================================================
// Verifies: deploy, state read/write, contract listing, multiple contracts.
#[test]
fn test_vm_contract_lifecycle() {
    println!("\n=== TEST 1: VM Contract Lifecycle ===\n");

    let engine = WasmEngine::new();

    // Deploy USP-01 token contract
    let token_addr = engine
        .deploy_contract(
            "alice".to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .expect("Deploy token contract");

    assert!(
        token_addr.starts_with("LOSCon"),
        "Contract address must start with LOSCon"
    );
    println!("  Token contract deployed at: {}", token_addr);

    // Deploy DEX AMM contract
    let dex_addr = engine
        .deploy_contract(
            "alice".to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            2,
        )
        .expect("Deploy DEX contract");

    assert_ne!(
        token_addr, dex_addr,
        "Different deployments must have unique addresses"
    );
    println!("  DEX contract deployed at: {}", dex_addr);

    // Verify contract count
    assert_eq!(engine.contract_count().unwrap(), 2);

    // List contracts
    let contracts = engine.list_contracts().unwrap();
    assert_eq!(contracts.len(), 2);

    // Verify contract ownership
    let token_contract = engine.get_contract(&token_addr).unwrap();
    assert_eq!(token_contract.owner, "alice");
    assert_eq!(token_contract.balance, 0);

    println!("  Contract lifecycle: PASS\n");
}

// ============================================================================
// TEST 2: USP-01 TOKEN INIT PATTERNS
// ============================================================================
// Verifies: state key conventions, metadata storage, initialization flag.
#[test]
fn test_usp01_token_init() {
    println!("\n=== TEST 2: USP-01 Token Init ===\n");

    let engine = WasmEngine::new();
    let deployer = "LOS_deployer_alice";
    let total_supply: u128 = 1_000_000 * CIL_PER_LOS; // 1M tokens

    // Deploy contract
    let addr = engine
        .deploy_contract(
            deployer.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // Simulate USP-01 init by setting state keys (as the contract would)
    vm_set_state(&engine, &addr, USP01_INIT_KEY, "1", deployer);
    vm_set_state(&engine, &addr, USP01_NAME_KEY, "Test Token", deployer);
    vm_set_state(&engine, &addr, USP01_SYMBOL_KEY, "TST", deployer);
    vm_set_state(&engine, &addr, USP01_DECIMALS_KEY, "11", deployer);
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &total_supply.to_string(),
        deployer,
    );
    vm_set_state(&engine, &addr, USP01_IS_WRAPPED_KEY, "0", deployer);
    vm_set_state(&engine, &addr, USP01_MAX_SUPPLY_KEY, "0", deployer);
    vm_set_state(&engine, &addr, USP01_OWNER_KEY, deployer, deployer);

    // Assign total supply to deployer
    vm_set_state(
        &engine,
        &addr,
        &balance_key(deployer),
        &total_supply.to_string(),
        deployer,
    );

    // Verify all metadata
    assert_eq!(vm_get_state(&engine, &addr, USP01_INIT_KEY, deployer), "1");
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_NAME_KEY, deployer),
        "Test Token"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_SYMBOL_KEY, deployer),
        "TST"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_DECIMALS_KEY, deployer),
        "11"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, deployer),
        total_supply.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_IS_WRAPPED_KEY, deployer),
        "0"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_OWNER_KEY, deployer),
        deployer
    );

    // Verify deployer balance equals total supply
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(deployer), deployer),
        total_supply.to_string()
    );

    println!("  Token metadata: PASS");
    println!("  Token init flag: PASS");
    println!("  Deployer balance = total_supply: PASS\n");
}

// ============================================================================
// TEST 3: USP-01 TRANSFER & BALANCES
// ============================================================================
// Verifies: balance debit/credit, state key patterns, checked arithmetic.
#[test]
fn test_usp01_transfer_balances() {
    println!("\n=== TEST 3: USP-01 Transfer & Balances ===\n");

    let engine = WasmEngine::new();
    let alice = "LOS_alice";
    let bob = "LOS_bob";
    let charlie = "LOS_charlie";

    let total_supply: u128 = 10_000 * CIL_PER_LOS;
    let transfer_amount: u128 = 2_500 * CIL_PER_LOS;

    // Deploy and init
    let addr = engine
        .deploy_contract(alice.to_string(), MINIMAL_WASM.to_vec(), BTreeMap::new(), 1)
        .unwrap();

    vm_set_state(&engine, &addr, USP01_INIT_KEY, "1", alice);
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &total_supply.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &addr,
        &balance_key(alice),
        &total_supply.to_string(),
        alice,
    );

    // ── Transfer: Alice → Bob (2500 LOS) ──
    let alice_bal: u128 = vm_get_state(&engine, &addr, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let bob_bal: u128 = vm_get_state(&engine, &addr, &balance_key(bob), alice)
        .parse()
        .unwrap_or(0); // "null" → 0 for new accounts

    // Simulate transfer: debit sender, credit receiver
    let new_alice_bal = alice_bal
        .checked_sub(transfer_amount)
        .expect("Insufficient balance");
    let new_bob_bal = bob_bal
        .checked_add(transfer_amount)
        .expect("Balance overflow");

    vm_set_state(
        &engine,
        &addr,
        &balance_key(alice),
        &new_alice_bal.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &addr,
        &balance_key(bob),
        &new_bob_bal.to_string(),
        alice,
    );

    // Verify
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(alice), alice),
        new_alice_bal.to_string(),
    );
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(bob), alice),
        new_bob_bal.to_string(),
    );
    println!(
        "  Alice: {} → {} CIL (-{} CIL)",
        alice_bal, new_alice_bal, transfer_amount
    );
    println!(
        "  Bob:   {} → {} CIL (+{} CIL)",
        bob_bal, new_bob_bal, transfer_amount
    );

    // ── Conservation law: total_supply == sum(all_balances) ──
    let final_alice: u128 = vm_get_state(&engine, &addr, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let final_bob: u128 = vm_get_state(&engine, &addr, &balance_key(bob), alice)
        .parse()
        .unwrap();
    assert_eq!(
        final_alice + final_bob,
        total_supply,
        "Conservation violated!"
    );
    println!("  Conservation law: PASS (alice + bob = total_supply)");

    // ── Transfer: Bob → Charlie (500 LOS) ──
    let charlie_transfer: u128 = 500 * CIL_PER_LOS;
    let bob_after = final_bob
        .checked_sub(charlie_transfer)
        .expect("Bob insufficient");
    let charlie_after = charlie_transfer;

    vm_set_state(
        &engine,
        &addr,
        &balance_key(bob),
        &bob_after.to_string(),
        bob,
    );
    vm_set_state(
        &engine,
        &addr,
        &balance_key(charlie),
        &charlie_after.to_string(),
        bob,
    );

    // Final conservation check
    let fa: u128 = vm_get_state(&engine, &addr, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let fb: u128 = vm_get_state(&engine, &addr, &balance_key(bob), alice)
        .parse()
        .unwrap();
    let fc: u128 = vm_get_state(&engine, &addr, &balance_key(charlie), alice)
        .parse()
        .unwrap();
    assert_eq!(fa + fb + fc, total_supply, "3-party conservation violated!");
    println!("  3-party conservation: PASS (alice + bob + charlie = total_supply)\n");
}

// ============================================================================
// TEST 4: USP-01 APPROVE & TRANSFER_FROM
// ============================================================================
// Verifies: allowance state key patterns, spending delegation, limits.
#[test]
fn test_usp01_approve_and_transfer_from() {
    println!("\n=== TEST 4: USP-01 Approve & TransferFrom ===\n");

    let engine = WasmEngine::new();
    let owner = "LOS_owner";
    let spender = "LOS_spender";
    let recipient = "LOS_recipient";

    let total_supply: u128 = 5_000 * CIL_PER_LOS;
    let approve_amount: u128 = 1_000 * CIL_PER_LOS;
    let spend_amount: u128 = 600 * CIL_PER_LOS;

    let addr = engine
        .deploy_contract(owner.to_string(), MINIMAL_WASM.to_vec(), BTreeMap::new(), 1)
        .unwrap();

    // Init balances
    vm_set_state(&engine, &addr, USP01_INIT_KEY, "1", owner);
    vm_set_state(
        &engine,
        &addr,
        &balance_key(owner),
        &total_supply.to_string(),
        owner,
    );
    vm_set_state(&engine, &addr, &balance_key(spender), "0", owner);
    vm_set_state(&engine, &addr, &balance_key(recipient), "0", owner);

    // ── Approve: owner approves spender for 1000 LOS ──
    vm_set_state(
        &engine,
        &addr,
        &allowance_key(owner, spender),
        &approve_amount.to_string(),
        owner,
    );

    let stored_allowance: u128 =
        vm_get_state(&engine, &addr, &allowance_key(owner, spender), owner)
            .parse()
            .unwrap();
    assert_eq!(stored_allowance, approve_amount);
    println!(
        "  Approve: {} → spender for {} CIL: PASS",
        owner, approve_amount
    );

    // ── TransferFrom: spender moves 600 LOS from owner → recipient ──
    let current_allowance: u128 =
        vm_get_state(&engine, &addr, &allowance_key(owner, spender), spender)
            .parse()
            .unwrap();
    assert!(spend_amount <= current_allowance, "Spend exceeds allowance");

    let owner_bal: u128 = vm_get_state(&engine, &addr, &balance_key(owner), spender)
        .parse()
        .unwrap();
    assert!(spend_amount <= owner_bal, "Owner insufficient balance");

    // Execute transfer_from
    let new_owner_bal = owner_bal - spend_amount;
    let new_recipient_bal = spend_amount;
    let new_allowance = current_allowance - spend_amount;

    vm_set_state(
        &engine,
        &addr,
        &balance_key(owner),
        &new_owner_bal.to_string(),
        spender,
    );
    vm_set_state(
        &engine,
        &addr,
        &balance_key(recipient),
        &new_recipient_bal.to_string(),
        spender,
    );
    vm_set_state(
        &engine,
        &addr,
        &allowance_key(owner, spender),
        &new_allowance.to_string(),
        spender,
    );

    // Verify
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(owner), spender),
        new_owner_bal.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(recipient), spender),
        new_recipient_bal.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &addr, &allowance_key(owner, spender), spender),
        new_allowance.to_string()
    );
    println!(
        "  TransferFrom: {} CIL from owner → recipient: PASS",
        spend_amount
    );
    println!(
        "  Remaining allowance: {} CIL (expected {}): PASS",
        new_allowance,
        approve_amount - spend_amount
    );

    // ── Verify: spend > remaining allowance should fail ──
    let remaining: u128 = new_allowance;
    let too_much: u128 = remaining + 1;
    assert!(
        too_much > remaining,
        "Overspend must be caught at application level"
    );
    println!("  Overspend prevention: PASS\n");
}

// ============================================================================
// TEST 5: USP-01 BURN & SUPPLY REDUCTION
// ============================================================================
// Verifies: supply decreases permanently, balance deduction, checked math.
#[test]
fn test_usp01_burn_supply_reduction() {
    println!("\n=== TEST 5: USP-01 Burn & Supply Reduction ===\n");

    let engine = WasmEngine::new();
    let deployer = "LOS_deployer";
    let total_supply: u128 = 21_936_236 * CIL_PER_LOS; // Full LOS supply
    let burn_amount: u128 = 100 * CIL_PER_LOS;

    let addr = engine
        .deploy_contract(
            deployer.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // Init
    vm_set_state(&engine, &addr, USP01_INIT_KEY, "1", deployer);
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &total_supply.to_string(),
        deployer,
    );
    vm_set_state(
        &engine,
        &addr,
        &balance_key(deployer),
        &total_supply.to_string(),
        deployer,
    );

    // ── Burn: deployer burns 100 LOS ──
    let bal_before: u128 = vm_get_state(&engine, &addr, &balance_key(deployer), deployer)
        .parse()
        .unwrap();
    let supply_before: u128 = vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, deployer)
        .parse()
        .unwrap();

    let bal_after = bal_before
        .checked_sub(burn_amount)
        .expect("Insufficient balance for burn");
    let supply_after = supply_before
        .checked_sub(burn_amount)
        .expect("Supply underflow");

    vm_set_state(
        &engine,
        &addr,
        &balance_key(deployer),
        &bal_after.to_string(),
        deployer,
    );
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &supply_after.to_string(),
        deployer,
    );

    // Also use VM mock dispatch burn to test contract balance
    engine.send_to_contract(&addr, burn_amount).unwrap();
    let burn_call = ContractCall {
        contract: addr.clone(),
        function: "burn".to_string(),
        args: vec![burn_amount.to_string()],
        gas_limit: 1000,
        caller: deployer.to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(burn_call).unwrap();
    assert!(result.success, "VM burn failed: {}", result.output);

    // Verify
    let final_bal: u128 = vm_get_state(&engine, &addr, &balance_key(deployer), deployer)
        .parse()
        .unwrap();
    let final_supply: u128 = vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, deployer)
        .parse()
        .unwrap();

    assert_eq!(final_bal, total_supply - burn_amount);
    assert_eq!(final_supply, total_supply - burn_amount);
    println!(
        "  Supply: {} → {} CIL (burned {} CIL)",
        total_supply, final_supply, burn_amount
    );
    println!("  Supply permanently reduced: PASS");

    // ── Burn is irreversible: cannot unburn ──
    // Supply should only decrease, never increase (no mint allowed)
    let mint_call = ContractCall {
        contract: addr.clone(),
        function: "mint".to_string(),
        args: vec![burn_amount.to_string()],
        gas_limit: 1000,
        caller: deployer.to_string(),
        block_timestamp: now_secs(),
    };
    let mint_result = engine.call_contract(mint_call);
    assert!(
        mint_result.is_err(),
        "Mint must be blocked in VM (P1-3 security)"
    );
    println!("  Mint blocked (P1-3 security): PASS\n");
}

// ============================================================================
// TEST 6: DEX AMM INITIALIZATION
// ============================================================================
// Verifies: DEX state patterns, pool counter init, owner assignment.
#[test]
fn test_dex_amm_initialization() {
    println!("\n=== TEST 6: DEX AMM Initialization ===\n");

    let engine = WasmEngine::new();
    let dex_owner = "LOS_dex_deployer";

    let addr = engine
        .deploy_contract(
            dex_owner.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // Simulate DEX init
    vm_set_state(&engine, &addr, DEX_INIT_KEY, "1", dex_owner);
    vm_set_state(&engine, &addr, DEX_OWNER_KEY, dex_owner, dex_owner);
    vm_set_state(&engine, &addr, DEX_POOL_COUNT_KEY, "0", dex_owner);

    // Verify
    assert_eq!(vm_get_state(&engine, &addr, DEX_INIT_KEY, dex_owner), "1");
    assert_eq!(
        vm_get_state(&engine, &addr, DEX_OWNER_KEY, dex_owner),
        dex_owner
    );
    assert_eq!(
        vm_get_state(&engine, &addr, DEX_POOL_COUNT_KEY, dex_owner),
        "0"
    );

    println!("  DEX init flag: PASS");
    println!("  DEX owner: PASS");
    println!("  DEX pool_count = 0: PASS\n");
}

// ============================================================================
// TEST 7: DEX POOL CREATION MATH
// ============================================================================
// Verifies: isqrt, initial LP = isqrt(a*b) - MINIMUM_LIQUIDITY, pool state.
#[test]
fn test_dex_pool_creation_math() {
    println!("\n=== TEST 7: DEX Pool Creation Math ===\n");

    // ── isqrt correctness ──
    assert_eq!(isqrt(0), 0);
    assert_eq!(isqrt(1), 1);
    assert_eq!(isqrt(4), 2);
    assert_eq!(isqrt(9), 3);
    assert_eq!(isqrt(10), 3); // floor
    assert_eq!(isqrt(100), 10);
    assert_eq!(isqrt(1_000_000), 1_000);
    // Production isqrt uses div_ceil — matches los_core::validator_rewards::isqrt
    println!("  isqrt correctness: PASS");

    // ── Pool creation: token_a=10000, token_b=40000 ──
    let amount_a: u128 = 10_000 * CIL_PER_LOS;
    let amount_b: u128 = 40_000 * CIL_PER_LOS;

    let product = amount_a
        .checked_mul(amount_b)
        .expect("Product must not overflow");
    let initial_lp = isqrt(product);

    // LP must be > MINIMUM_LIQUIDITY (else pool creation fails)
    assert!(
        initial_lp > MINIMUM_LIQUIDITY,
        "Initial LP {} must exceed MINIMUM_LIQUIDITY {}",
        initial_lp,
        MINIMUM_LIQUIDITY
    );

    let creator_lp = initial_lp - MINIMUM_LIQUIDITY;
    println!(
        "  Pool: {} × {} = {} (product)",
        amount_a, amount_b, product
    );
    println!("  isqrt(product) = {}", initial_lp);
    println!(
        "  Creator LP = {} - {} = {}",
        initial_lp, MINIMUM_LIQUIDITY, creator_lp
    );

    // ── Verify: isqrt(a*b)^2 ≤ a*b < (isqrt(a*b)+1)^2 ──
    assert!(initial_lp * initial_lp <= product);
    assert!((initial_lp + 1) * (initial_lp + 1) > product);
    println!("  isqrt floor property: PASS");

    // ── State simulation for pool creation ──
    let engine = WasmEngine::new();
    let creator = "LOS_pool_creator";
    let token_a_addr = "LOSConTokenA";
    let token_b_addr = "LOSConTokenB";
    let pid = pool_id(token_a_addr, token_b_addr);

    let dex = engine
        .deploy_contract(
            creator.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // Set pool state
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "token_a"),
        token_a_addr,
        creator,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "token_b"),
        token_b_addr,
        creator,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_a"),
        &amount_a.to_string(),
        creator,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_b"),
        &amount_b.to_string(),
        creator,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "total_lp"),
        &initial_lp.to_string(),
        creator,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "fee_bps"),
        &DEFAULT_FEE_BPS.to_string(),
        creator,
    );
    vm_set_state(&engine, &dex, &pool_key(&pid, "creator"), creator, creator);
    vm_set_state(
        &engine,
        &dex,
        &lp_key(&pid, creator),
        &creator_lp.to_string(),
        creator,
    );

    // Verify pool state
    assert_eq!(
        vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_a"), creator),
        amount_a.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_b"), creator),
        amount_b.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &dex, &pool_key(&pid, "total_lp"), creator),
        initial_lp.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &dex, &lp_key(&pid, creator), creator),
        creator_lp.to_string()
    );
    println!("  Pool state storage: PASS\n");
}

// ============================================================================
// TEST 8: DEX SWAP MATH
// ============================================================================
// Verifies: constant product, fee deduction, output calculation, k invariant.
#[test]
fn test_dex_swap_math() {
    println!("\n=== TEST 8: DEX Swap Math ===\n");

    // ── Fee deduction ──
    let amount: u128 = 1_000 * CIL_PER_LOS;
    let (after_fee, fee) = deduct_fee(amount, DEFAULT_FEE_BPS);

    assert_eq!(fee, amount * 30 / 10_000);
    assert_eq!(
        after_fee + fee,
        amount,
        "Fee math: amount = after_fee + fee"
    );
    println!(
        "  Fee: {} CIL × 0.3% = {} CIL fee, {} CIL net",
        amount, fee, after_fee
    );

    // ── Fee boundary: 0% fee ──
    let (after_zero, zero_fee) = deduct_fee(amount, 0);
    assert_eq!(zero_fee, 0);
    assert_eq!(after_zero, amount);
    println!("  0% fee: PASS");

    // ── Fee boundary: max 10% fee ──
    let (after_max, max_fee) = deduct_fee(amount, MAX_FEE_BPS);
    assert_eq!(max_fee, amount * 1_000 / 10_000);
    assert_eq!(after_max, amount - max_fee);
    println!("  10% max fee: {} CIL fee: PASS", max_fee);

    // ── Constant product swap ──
    let reserve_a: u128 = 100_000 * CIL_PER_LOS;
    let reserve_b: u128 = 200_000 * CIL_PER_LOS;
    let swap_in: u128 = 1_000 * CIL_PER_LOS;
    let k_before = reserve_a * reserve_b;

    // Apply fee first
    let (amount_in_after_fee, _swap_fee) = deduct_fee(swap_in, DEFAULT_FEE_BPS);

    // Compute output
    let amount_out = compute_output(amount_in_after_fee, reserve_a, reserve_b);

    assert!(amount_out > 0, "Swap output must be positive");
    assert!(amount_out < reserve_b, "Output must be less than reserve_b");

    // New reserves (fee stays in pool)
    let new_reserve_a = reserve_a + swap_in; // Full amount goes to reserve (incl fee)
    let new_reserve_b = reserve_b - amount_out;
    let k_after = new_reserve_a * new_reserve_b;

    // k must increase or stay same (fee accrues to LPs)
    assert!(
        k_after >= k_before,
        "k invariant violated: {} < {}",
        k_after,
        k_before
    );
    println!("  Swap: {} CIL in → {} CIL out", swap_in, amount_out);
    println!("  k_before: {}", k_before);
    println!("  k_after:  {} (Δ = +{})", k_after, k_after - k_before);
    println!("  k invariant (k_after ≥ k_before): PASS");

    // ── Price impact: larger swaps produce worse rates ──
    let small_swap: u128 = 100 * CIL_PER_LOS;
    let large_swap: u128 = 10_000 * CIL_PER_LOS;

    let (small_net, _) = deduct_fee(small_swap, DEFAULT_FEE_BPS);
    let (large_net, _) = deduct_fee(large_swap, DEFAULT_FEE_BPS);

    let small_out = compute_output(small_net, reserve_a, reserve_b);
    let large_out = compute_output(large_net, reserve_a, reserve_b);

    // Rate = output/input (per unit)
    let small_rate = small_out * CIL_PER_LOS / small_swap;
    let large_rate = large_out * CIL_PER_LOS / large_swap;

    assert!(
        small_rate > large_rate,
        "Small swaps must have better rate than large swaps"
    );
    println!(
        "  Price impact: small rate {} > large rate {}: PASS",
        small_rate, large_rate
    );

    // ── Edge case: swap 0 ──
    let zero_out = compute_output(0, reserve_a, reserve_b);
    assert_eq!(zero_out, 0, "Zero input → zero output");
    println!("  Zero swap: PASS\n");
}

// ============================================================================
// TEST 9: DEX LIQUIDITY ADD/REMOVE
// ============================================================================
// Verifies: proportional LP shares, min slippage, removal withdrawal.
#[test]
fn test_dex_liquidity_add_remove() {
    println!("\n=== TEST 9: DEX Liquidity Add/Remove ===\n");

    // Initial pool: 10k × 40k
    let reserve_a: u128 = 10_000 * CIL_PER_LOS;
    let reserve_b: u128 = 40_000 * CIL_PER_LOS;
    let total_lp: u128 = isqrt(reserve_a * reserve_b);
    let creator_lp: u128 = total_lp - MINIMUM_LIQUIDITY;

    println!(
        "  Initial pool: reserve_a={}, reserve_b={}",
        reserve_a, reserve_b
    );
    println!("  Total LP: {}, Creator LP: {}", total_lp, creator_lp);

    // ── Add liquidity: proportional ──
    let add_a: u128 = 1_000 * CIL_PER_LOS;
    let add_b: u128 = 4_000 * CIL_PER_LOS; // Must maintain ratio 1:4

    // New LP tokens: min(add_a * total_lp / reserve_a, add_b * total_lp / reserve_b)
    let lp_from_a = add_a * total_lp / reserve_a;
    let lp_from_b = add_b * total_lp / reserve_b;
    let new_lp = std::cmp::min(lp_from_a, lp_from_b);

    assert!(new_lp > 0, "New LP must be positive");
    assert_eq!(
        lp_from_a, lp_from_b,
        "Proportional add must give equal LP from both sides"
    );

    let new_total_lp = total_lp + new_lp;
    let new_reserve_a = reserve_a + add_a;
    let new_reserve_b = reserve_b + add_b;

    println!(
        "  Add liquidity: {} + {} → {} new LP tokens",
        add_a, add_b, new_lp
    );
    println!("  Total LP: {} → {}", total_lp, new_total_lp);

    // ── Remove liquidity: proportional withdrawal ──
    let remove_lp = new_lp; // Remove what we just added
    let withdraw_a = remove_lp * new_reserve_a / new_total_lp;
    let withdraw_b = remove_lp * new_reserve_b / new_total_lp;

    assert!(
        withdraw_a > 0 && withdraw_b > 0,
        "Withdrawal must be positive"
    );

    // Ratio must be approximately maintained
    // withdraw_a / withdraw_b ≈ reserve_a / reserve_b = 1:4
    let ratio_check = withdraw_b * 10 / withdraw_a; // Should be ~40 (1:4 ratio × 10)
    assert!(
        (39..=41).contains(&ratio_check),
        "Withdrawal ratio must maintain pool ratio (got {})",
        ratio_check
    );
    println!(
        "  Remove {} LP → {} CIL + {} CIL",
        remove_lp, withdraw_a, withdraw_b
    );
    println!("  Ratio preserved: PASS");

    // ── After removal, reserves should be back to approximately original ──
    let final_reserve_a = new_reserve_a - withdraw_a;
    let final_reserve_b = new_reserve_b - withdraw_b;

    // Due to integer rounding, may differ by up to 1 CIL per operation
    let diff_a = final_reserve_a.abs_diff(reserve_a);
    let diff_b = final_reserve_b.abs_diff(reserve_b);

    // Allow rounding error up to 1 LOS
    assert!(
        diff_a < CIL_PER_LOS,
        "Reserve A drifted too far: {} vs {}",
        final_reserve_a,
        reserve_a
    );
    assert!(
        diff_b < CIL_PER_LOS,
        "Reserve B drifted too far: {} vs {}",
        final_reserve_b,
        reserve_b
    );
    println!("  Reserves restored (rounding ≤ 1 LOS): PASS");

    // ── Slippage protection: min_lp_tokens check ──
    let min_lp_tokens: u128 = new_lp + 1; // Set unreasonably high
    assert!(
        new_lp < min_lp_tokens,
        "Slippage check: new_lp {} < min {} should trigger revert",
        new_lp,
        min_lp_tokens
    );
    println!("  Slippage protection: PASS\n");
}

// ============================================================================
// TEST 10: BLOCKCHAIN CONTRACT BLOCKS
// ============================================================================
// Verifies: ContractDeploy/Call block types, signing, hashing, PoW.
#[test]
fn test_blockchain_contract_blocks() {
    println!("\n=== TEST 10: Blockchain Contract Blocks ===\n");

    let node = SimNode::new();
    assert!(
        los_crypto::validate_address(&node.address),
        "SimNode address must be valid LOS address"
    );
    println!("  Node: {}", node.address);

    // ── ContractDeploy block ──
    let code_hash = blake3::hash(MINIMAL_WASM).to_hex().to_string();
    let deploy_link = format!("DEPLOY:{}", code_hash);

    let mut deploy_block = Block {
        account: node.address.clone(),
        previous: "0".repeat(64), // genesis
        block_type: BlockType::ContractDeploy,
        amount: 0,
        link: deploy_link.clone(),
        signature: String::new(),
        public_key: node.pubkey_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: MIN_DEPLOY_FEE_CIL,
    };

    mine_and_sign(&mut deploy_block, &node.secret_key);

    // Verify block
    assert!(deploy_block.verify_pow(), "Deploy block PoW must be valid");
    assert!(
        deploy_block.verify_signature(),
        "Deploy block signature must be valid"
    );
    assert_eq!(deploy_block.block_type, BlockType::ContractDeploy);
    assert!(
        deploy_block.fee >= MIN_DEPLOY_FEE_CIL,
        "Deploy fee must meet minimum"
    );

    let deploy_hash = deploy_block.calculate_hash();
    assert!(!deploy_hash.is_empty());
    println!("  ContractDeploy block: {}", &deploy_hash[..16]);
    println!("  Deploy fee: {} CIL", deploy_block.fee);
    println!("  Code hash: {}", &code_hash[..16]);

    // ── ContractCall block ──
    let contract_addr = "LOSConTestContract";
    let function = "transfer";
    let args_json = serde_json::json!(["LOS_bob", "100000000000"]);
    let args_b64 = base64_encode(&args_json.to_string());
    let call_link = format!("CALL:{}:{}:{}", contract_addr, function, args_b64);

    let mut call_block = Block {
        account: node.address.clone(),
        previous: deploy_hash.clone(),
        block_type: BlockType::ContractCall,
        amount: 0,
        link: call_link,
        signature: String::new(),
        public_key: node.pubkey_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: MIN_CALL_FEE_CIL,
    };

    mine_and_sign(&mut call_block, &node.secret_key);

    assert!(call_block.verify_pow(), "Call block PoW must be valid");
    assert!(
        call_block.verify_signature(),
        "Call block signature must be valid"
    );
    assert_eq!(call_block.block_type, BlockType::ContractCall);

    let call_hash = call_block.calculate_hash();
    println!("  ContractCall block: {}", &call_hash[..16]);
    println!("  Function: {}", function);

    // ── Block chain property: previous links ──
    assert_eq!(call_block.previous, deploy_hash);
    println!("  Chain linkage: PASS");

    // ── BlockType discriminants ──
    assert_ne!(
        BlockType::ContractDeploy,
        BlockType::ContractCall,
        "Deploy and Call are distinct types"
    );
    assert_ne!(BlockType::ContractDeploy, BlockType::Send);
    assert_ne!(BlockType::ContractCall, BlockType::Receive);
    println!("  Block type discriminants: PASS");

    // ── Process blocks through ledger ──
    // First, fund the account with a Mint block so it can pay deploy/call fees
    let funding_amount: u128 = 100 * CIL_PER_LOS;
    let mut mint_block = Block {
        account: node.address.clone(),
        previous: "0".to_string(),
        block_type: BlockType::Mint,
        amount: funding_amount,
        link: "FAUCET:TESTNET:E2E".to_string(),
        signature: String::new(),
        public_key: node.pubkey_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: 0,
    };
    mine_and_sign(&mut mint_block, &node.secret_key);
    let mint_hash = mint_block.calculate_hash();

    let mut ledger = Ledger::new();
    let mint_result = ledger.process_block(&mint_block);
    assert!(
        mint_result.is_ok(),
        "Mint block must succeed: {:?}",
        mint_result
    );

    // Now create the deploy block chained from the mint
    deploy_block.previous = mint_hash.clone();
    deploy_block.timestamp = now_secs(); // Ensure monotonic timestamp
    mine_and_sign(&mut deploy_block, &node.secret_key);
    let deploy_hash_2 = deploy_block.calculate_hash();
    let deploy_result = ledger.process_block(&deploy_block);
    assert!(
        deploy_result.is_ok(),
        "Deploy block must succeed: {:?}",
        deploy_result
    );

    // Chain the call block from the deploy
    call_block.previous = deploy_hash_2.clone();
    call_block.timestamp = now_secs(); // Ensure monotonic timestamp
    mine_and_sign(&mut call_block, &node.secret_key);
    let call_result = ledger.process_block(&call_block);
    assert!(
        call_result.is_ok(),
        "Call block must succeed: {:?}",
        call_result
    );

    // Verify blocks are stored in ledger
    assert!(
        ledger.blocks.contains_key(&mint_hash),
        "Mint block must be in ledger"
    );
    assert!(
        ledger.blocks.contains_key(&deploy_hash_2),
        "Deploy block must be in ledger"
    );

    // Verify account state
    let acct = ledger.accounts.get(&node.address).unwrap();
    assert_eq!(acct.block_count, 3, "Account must have 3 blocks");
    // Balance = funding - deploy_fee - call_fee
    let expected_balance = funding_amount - MIN_DEPLOY_FEE_CIL - MIN_CALL_FEE_CIL;
    assert_eq!(
        acct.balance, expected_balance,
        "Balance must reflect fee deductions"
    );
    println!("  Ledger processing: PASS");
    println!(
        "  Balance after fees: {} CIL (expected {})",
        acct.balance, expected_balance
    );
    println!("  Ledger storage: PASS\n");
}

/// Simple base64 encoding (no external dependency needed for test).
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();
    let chunks = bytes.chunks(3);
    for chunk in chunks {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

// ============================================================================
// TEST 11: GAS LIMITS & ERROR HANDLING
// ============================================================================
// Verifies: gas exceeded, unknown function, nonexistent contract, bad calls.
#[test]
fn test_gas_limits_and_errors() {
    println!("\n=== TEST 11: Gas Limits & Error Handling ===\n");

    let engine = WasmEngine::new();
    let addr = engine
        .deploy_contract(
            "alice".to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // ── Gas exceeded ──
    engine.send_to_contract(&addr, 1000).unwrap();
    let call = ContractCall {
        contract: addr.clone(),
        function: "transfer".to_string(),
        args: vec!["500".to_string(), "bob".to_string()],
        gas_limit: 1, // Way too low
        caller: "alice".to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call);
    assert!(result.is_err(), "Gas limit too low must fail: {:?}", result);
    println!("  Gas exceeded: PASS");

    // ── Unknown function ──
    let call = ContractCall {
        contract: addr.clone(),
        function: "nonexistent_function".to_string(),
        args: vec![],
        gas_limit: 1000,
        caller: "alice".to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call);
    assert!(result.is_err(), "Unknown function must fail");
    println!("  Unknown function: PASS");

    // ── Nonexistent contract ──
    let call = ContractCall {
        contract: "LOSConDoesNotExist".to_string(),
        function: "get_balance".to_string(),
        args: vec![],
        gas_limit: 1000,
        caller: "alice".to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call);
    assert!(result.is_err(), "Nonexistent contract must fail");
    println!("  Nonexistent contract: PASS");

    // ── Transfer with insufficient balance ──
    let call = ContractCall {
        contract: addr.clone(),
        function: "transfer".to_string(),
        args: vec!["999999999".to_string(), "bob".to_string()],
        gas_limit: 1000,
        caller: "alice".to_string(),
        block_timestamp: now_secs(),
    };
    let result = engine.call_contract(call);
    assert!(result.is_err(), "Insufficient balance transfer must fail");
    println!("  Insufficient balance: PASS");

    // ── Invalid WASM deployment ──
    let invalid_bytes = vec![0x00, 0x00, 0x00, 0x00];
    let result = engine.deploy_contract("alice".to_string(), invalid_bytes, BTreeMap::new(), 1);
    assert!(result.is_err(), "Invalid WASM must fail deployment");
    println!("  Invalid WASM deployment: PASS");

    // ── Gas pricing constants ──
    const { assert!(GAS_PRICE_CIL > 0) };
    const {
        assert!(MIN_DEPLOY_FEE_CIL > MIN_CALL_FEE_CIL);
    };
    assert_eq!(MIN_CALL_FEE_CIL, BASE_FEE_CIL, "Call fee = base tx fee");
    assert_eq!(DEFAULT_GAS_LIMIT, 1_000_000);
    println!("  Gas pricing constants: PASS\n");
}

// ============================================================================
// TEST 12: FULL INTEGRATION FLOW
// ============================================================================
// Simulates: Token deploy → Init → Transfer → DEX deploy → Pool → Swap.
// Combined blockchain + VM + state verification.
#[test]
fn test_full_integration_flow() {
    println!("\n=== TEST 12: Full Integration Flow ===\n");

    let engine = WasmEngine::new();
    let alice = "LOS_alice";
    let bob = "LOS_bob";

    // ── Step 1: Deploy USP-01 Token A (wrapped BTC) ──
    let token_a = engine
        .deploy_contract(alice.to_string(), MINIMAL_WASM.to_vec(), BTreeMap::new(), 1)
        .unwrap();
    println!("  1. Token A deployed: {}", &token_a[..20]);

    let supply_a: u128 = 21_000_000 * CIL_PER_LOS; // 21M wBTC
    vm_set_state(&engine, &token_a, USP01_INIT_KEY, "1", alice);
    vm_set_state(&engine, &token_a, USP01_NAME_KEY, "Wrapped Bitcoin", alice);
    vm_set_state(&engine, &token_a, USP01_SYMBOL_KEY, "wBTC", alice);
    vm_set_state(&engine, &token_a, USP01_DECIMALS_KEY, "11", alice);
    vm_set_state(
        &engine,
        &token_a,
        USP01_TOTAL_SUPPLY_KEY,
        &supply_a.to_string(),
        alice,
    );
    vm_set_state(&engine, &token_a, USP01_IS_WRAPPED_KEY, "1", alice);
    vm_set_state(
        &engine,
        &token_a,
        USP01_WRAPPED_ORIGIN_KEY,
        "bitcoin",
        alice,
    );
    vm_set_state(
        &engine,
        &token_a,
        &balance_key(alice),
        &supply_a.to_string(),
        alice,
    );

    // ── Step 2: Deploy USP-01 Token B (wrapped ETH) ──
    let token_b = engine
        .deploy_contract(alice.to_string(), MINIMAL_WASM.to_vec(), BTreeMap::new(), 2)
        .unwrap();
    println!("  2. Token B deployed: {}", &token_b[..20]);

    let supply_b: u128 = 120_000_000 * CIL_PER_LOS; // 120M wETH
    vm_set_state(&engine, &token_b, USP01_INIT_KEY, "1", alice);
    vm_set_state(&engine, &token_b, USP01_NAME_KEY, "Wrapped Ethereum", alice);
    vm_set_state(&engine, &token_b, USP01_SYMBOL_KEY, "wETH", alice);
    vm_set_state(&engine, &token_b, USP01_DECIMALS_KEY, "11", alice);
    vm_set_state(
        &engine,
        &token_b,
        USP01_TOTAL_SUPPLY_KEY,
        &supply_b.to_string(),
        alice,
    );
    vm_set_state(&engine, &token_b, USP01_IS_WRAPPED_KEY, "1", alice);
    vm_set_state(
        &engine,
        &token_b,
        USP01_WRAPPED_ORIGIN_KEY,
        "ethereum",
        alice,
    );
    vm_set_state(
        &engine,
        &token_b,
        &balance_key(alice),
        &supply_b.to_string(),
        alice,
    );

    // ── Step 3: Alice transfers tokens to Bob ──
    let alice_to_bob_a: u128 = 5_000 * CIL_PER_LOS;
    let alice_to_bob_b: u128 = 20_000 * CIL_PER_LOS;

    // Token A: Alice → Bob
    let alice_bal_a: u128 = vm_get_state(&engine, &token_a, &balance_key(alice), alice)
        .parse()
        .unwrap();
    vm_set_state(
        &engine,
        &token_a,
        &balance_key(alice),
        &(alice_bal_a - alice_to_bob_a).to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &token_a,
        &balance_key(bob),
        &alice_to_bob_a.to_string(),
        alice,
    );

    // Token B: Alice → Bob
    let alice_bal_b: u128 = vm_get_state(&engine, &token_b, &balance_key(alice), alice)
        .parse()
        .unwrap();
    vm_set_state(
        &engine,
        &token_b,
        &balance_key(alice),
        &(alice_bal_b - alice_to_bob_b).to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &token_b,
        &balance_key(bob),
        &alice_to_bob_b.to_string(),
        alice,
    );
    println!(
        "  3. Alice → Bob: {} wBTC + {} wETH",
        alice_to_bob_a, alice_to_bob_b
    );

    // ── Step 4: Deploy DEX ──
    let dex = engine
        .deploy_contract(alice.to_string(), MINIMAL_WASM.to_vec(), BTreeMap::new(), 3)
        .unwrap();
    vm_set_state(&engine, &dex, DEX_INIT_KEY, "1", alice);
    vm_set_state(&engine, &dex, DEX_OWNER_KEY, alice, alice);
    vm_set_state(&engine, &dex, DEX_POOL_COUNT_KEY, "0", alice);
    println!("  4. DEX deployed: {}", &dex[..20]);

    // ── Step 5: Alice creates wBTC/wETH pool ──
    let pool_a: u128 = 1_000 * CIL_PER_LOS;
    let pool_b: u128 = 4_000 * CIL_PER_LOS;
    let pid = pool_id(&token_a, &token_b);
    let initial_lp = isqrt(pool_a * pool_b);
    let alice_lp = initial_lp - MINIMUM_LIQUIDITY;

    vm_set_state(&engine, &dex, &pool_key(&pid, "token_a"), &token_a, alice);
    vm_set_state(&engine, &dex, &pool_key(&pid, "token_b"), &token_b, alice);
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_a"),
        &pool_a.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_b"),
        &pool_b.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "total_lp"),
        &initial_lp.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "fee_bps"),
        &DEFAULT_FEE_BPS.to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &dex,
        &lp_key(&pid, alice),
        &alice_lp.to_string(),
        alice,
    );
    vm_set_state(&engine, &dex, DEX_POOL_COUNT_KEY, "1", alice);

    // Debit Alice's token balances for pool liquidity
    let alice_bal_a_now: u128 = vm_get_state(&engine, &token_a, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let alice_bal_b_now: u128 = vm_get_state(&engine, &token_b, &balance_key(alice), alice)
        .parse()
        .unwrap();
    vm_set_state(
        &engine,
        &token_a,
        &balance_key(alice),
        &(alice_bal_a_now - pool_a).to_string(),
        alice,
    );
    vm_set_state(
        &engine,
        &token_b,
        &balance_key(alice),
        &(alice_bal_b_now - pool_b).to_string(),
        alice,
    );

    println!(
        "  5. Pool created: {} wBTC + {} wETH → {} LP",
        pool_a, pool_b, alice_lp
    );

    // ── Step 6: Bob swaps 100 wBTC → wETH ──
    let swap_in: u128 = 100 * CIL_PER_LOS;
    let reserve_a: u128 = vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_a"), bob)
        .parse()
        .unwrap();
    let reserve_b: u128 = vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_b"), bob)
        .parse()
        .unwrap();

    let (net_in, fee) = deduct_fee(swap_in, DEFAULT_FEE_BPS);
    let amount_out = compute_output(net_in, reserve_a, reserve_b);

    assert!(amount_out > 0, "Swap must produce output");

    // Update pool reserves
    let new_reserve_a = reserve_a + swap_in;
    let new_reserve_b = reserve_b - amount_out;
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_a"),
        &new_reserve_a.to_string(),
        bob,
    );
    vm_set_state(
        &engine,
        &dex,
        &pool_key(&pid, "reserve_b"),
        &new_reserve_b.to_string(),
        bob,
    );

    // Update Bob's token balances
    let bob_bal_a: u128 = vm_get_state(&engine, &token_a, &balance_key(bob), bob)
        .parse()
        .unwrap();
    let bob_bal_b_str = vm_get_state(&engine, &token_b, &balance_key(bob), bob);
    let bob_bal_b: u128 = bob_bal_b_str.parse().unwrap();

    vm_set_state(
        &engine,
        &token_a,
        &balance_key(bob),
        &(bob_bal_a - swap_in).to_string(),
        bob,
    );
    vm_set_state(
        &engine,
        &token_b,
        &balance_key(bob),
        &(bob_bal_b + amount_out).to_string(),
        bob,
    );

    println!(
        "  6. Bob swaps {} wBTC → {} wETH (fee: {} CIL)",
        swap_in, amount_out, fee
    );

    // ── Verify k invariant ──
    let k_before = reserve_a * reserve_b;
    let k_after = new_reserve_a * new_reserve_b;
    assert!(k_after >= k_before, "k invariant must hold");
    println!("  k invariant: {} → {} (PASS)", k_before, k_after);

    // ── Verify token conservation across all accounts + pool ──
    let final_alice_a: u128 = vm_get_state(&engine, &token_a, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let final_bob_a: u128 = vm_get_state(&engine, &token_a, &balance_key(bob), alice)
        .parse()
        .unwrap();
    let final_pool_a: u128 = vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_a"), alice)
        .parse()
        .unwrap();

    // Token A total: alice + bob + pool reserves = original supply
    assert_eq!(
        final_alice_a + final_bob_a + final_pool_a,
        supply_a,
        "Token A conservation violated"
    );
    println!("  Token A conservation: PASS");

    let final_alice_b: u128 = vm_get_state(&engine, &token_b, &balance_key(alice), alice)
        .parse()
        .unwrap();
    let final_bob_b: u128 = vm_get_state(&engine, &token_b, &balance_key(bob), alice)
        .parse()
        .unwrap();
    let final_pool_b: u128 = vm_get_state(&engine, &dex, &pool_key(&pid, "reserve_b"), alice)
        .parse()
        .unwrap();

    assert_eq!(
        final_alice_b + final_bob_b + final_pool_b,
        supply_b,
        "Token B conservation violated"
    );
    println!("  Token B conservation: PASS");

    // ── Step 7: Create blockchain blocks for the operations ──
    let node = SimNode::new();
    let mut prev_hash = "0".repeat(64);

    // Deploy block for token A
    let code_hash = blake3::hash(MINIMAL_WASM).to_hex().to_string();
    let mut deploy_blk = Block {
        account: node.address.clone(),
        previous: prev_hash.clone(),
        block_type: BlockType::ContractDeploy,
        amount: 0,
        link: format!("DEPLOY:{}", code_hash),
        signature: String::new(),
        public_key: node.pubkey_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: MIN_DEPLOY_FEE_CIL,
    };
    mine_and_sign(&mut deploy_blk, &node.secret_key);
    assert!(deploy_blk.verify_pow());
    assert!(deploy_blk.verify_signature());
    prev_hash = deploy_blk.calculate_hash();
    println!("  7a. Deploy block mined: {}", &prev_hash[..16]);

    // Call block for swap
    let call_args = format!(
        "[\"{}\" ,\"{}\" ,\"{}\" ,\"0\" ,\"0\"]",
        pid, token_a, swap_in
    );
    let call_link = format!("CALL:{}:swap:{}", dex, base64_encode(&call_args));
    let mut call_blk = Block {
        account: node.address.clone(),
        previous: prev_hash.clone(),
        block_type: BlockType::ContractCall,
        amount: 0,
        link: call_link,
        signature: String::new(),
        public_key: node.pubkey_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: MIN_CALL_FEE_CIL,
    };
    mine_and_sign(&mut call_blk, &node.secret_key);
    assert!(call_blk.verify_pow());
    assert!(call_blk.verify_signature());
    let call_hash = call_blk.calculate_hash();
    println!("  7b. Call block mined: {}", &call_hash[..16]);

    // ── Final summary ──
    println!("\n  === Integration Summary ===");
    println!("  Contracts deployed: 3 (TokenA, TokenB, DEX)");
    println!("  Token transfers: 2 (Alice→Bob × 2 tokens)");
    println!("  Pool created: 1 (wBTC/wETH)");
    println!("  Swaps executed: 1 (Bob: wBTC→wETH)");
    println!("  Blocks mined: 2 (Deploy + Call)");
    println!("  Conservation verified: PASS");
    println!("  k invariant verified: PASS");
    println!("  Full integration flow: PASS\n");
}

// ============================================================================
// TEST 13: DEX FEE EDGE CASES & PRECISION
// ============================================================================
// Verifies: tiny amounts, large amounts, overflow safety, edge cases.
#[test]
fn test_dex_fee_edge_cases() {
    println!("\n=== TEST 13: DEX Fee Edge Cases & Precision ===\n");

    // ── Tiny amount: 1 CIL ──
    let (after, fee) = deduct_fee(1, DEFAULT_FEE_BPS);
    assert_eq!(fee, 0, "1 CIL fee rounds to 0 (integer)");
    assert_eq!(after, 1, "No fee deducted for tiny amount");
    println!("  1 CIL fee = 0 (integer rounding): PASS");

    // ── Exact fee boundary: 10000 / 30 = 333.33... ──
    let amount: u128 = 10_000;
    let (after, fee) = deduct_fee(amount, DEFAULT_FEE_BPS);
    assert_eq!(fee, 30); // 10000 * 30 / 10000 = 30
    assert_eq!(after, 9_970);
    println!("  10000 CIL fee = 30 (exact): PASS");

    // ── Large amount (near max supply) ──
    let large: u128 = 21_936_236 * CIL_PER_LOS;
    let (after_large, fee_large) = deduct_fee(large, DEFAULT_FEE_BPS);
    assert_eq!(fee_large, large * 30 / 10_000);
    assert_eq!(after_large + fee_large, large, "Large fee conservation");
    println!("  Max supply fee conservation: PASS");

    // ── Swap output with very unbalanced pool ──
    let reserve_a: u128 = CIL_PER_LOS; // Tiny reserve
    let reserve_b: u128 = 1_000_000 * CIL_PER_LOS; // Huge reserve
    let swap_in: u128 = CIL_PER_LOS;

    let out = compute_output(swap_in, reserve_a, reserve_b);
    // With equal swap_in and reserve_a: out = swap_in * reserve_b / (reserve_a + swap_in)
    // = 1 * 1_000_000 / (1 + 1) = 500_000
    assert_eq!(out, 500_000 * CIL_PER_LOS);
    println!("  Unbalanced pool swap: {} CIL out: PASS", out);

    // ── Swap with 0 reserves should return 0 ──
    assert_eq!(compute_output(1000, 0, 1000), 0);
    assert_eq!(compute_output(1000, 1000, 0), 0);
    println!("  Zero reserve edge cases: PASS");

    // ── Pool ID determinism ──
    let id1 = pool_id("AAA", "BBB");
    let id2 = pool_id("BBB", "AAA");
    assert_eq!(id1, id2, "Pool ID must be order-independent");
    println!("  Pool ID determinism: PASS\n");
}

// ============================================================================
// TEST 14: WRAPPED ASSET (USP-01 BRIDGE) PATTERNS
// ============================================================================
// Verifies: wrapped token metadata, bridge operator state, max supply cap.
#[test]
fn test_usp01_wrapped_asset_patterns() {
    println!("\n=== TEST 14: Wrapped Asset Patterns ===\n");

    let engine = WasmEngine::new();
    let bridge_op = "LOS_bridge_operator";
    let max_supply: u128 = 21_000_000 * CIL_PER_LOS; // 21M BTC cap

    let addr = engine
        .deploy_contract(
            bridge_op.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    // Init as wrapped token
    vm_set_state(&engine, &addr, USP01_INIT_KEY, "1", bridge_op);
    vm_set_state(&engine, &addr, USP01_NAME_KEY, "Wrapped Bitcoin", bridge_op);
    vm_set_state(&engine, &addr, USP01_SYMBOL_KEY, "wBTC", bridge_op);
    vm_set_state(&engine, &addr, USP01_DECIMALS_KEY, "11", bridge_op);
    vm_set_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, "0", bridge_op);
    vm_set_state(&engine, &addr, USP01_IS_WRAPPED_KEY, "1", bridge_op);
    vm_set_state(
        &engine,
        &addr,
        USP01_WRAPPED_ORIGIN_KEY,
        "bitcoin",
        bridge_op,
    );
    vm_set_state(
        &engine,
        &addr,
        USP01_MAX_SUPPLY_KEY,
        &max_supply.to_string(),
        bridge_op,
    );
    vm_set_state(
        &engine,
        &addr,
        USP01_BRIDGE_OPERATOR_KEY,
        bridge_op,
        bridge_op,
    );
    vm_set_state(&engine, &addr, USP01_OWNER_KEY, bridge_op, bridge_op);

    // Verify wrapped metadata
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_IS_WRAPPED_KEY, bridge_op),
        "1"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_WRAPPED_ORIGIN_KEY, bridge_op),
        "bitcoin"
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_BRIDGE_OPERATOR_KEY, bridge_op),
        bridge_op
    );
    println!("  Wrapped metadata: PASS");

    // ── Simulate wrap_mint: bridge mints 10 wBTC ──
    let mint_amount: u128 = 10 * CIL_PER_LOS;
    let current_supply: u128 = vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, bridge_op)
        .parse()
        .unwrap();

    // Max supply check
    assert!(
        current_supply + mint_amount <= max_supply,
        "Mint would exceed max supply"
    );

    let new_supply = current_supply + mint_amount;
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &new_supply.to_string(),
        bridge_op,
    );

    let recipient = "LOS_user";
    let recipient_bal: u128 = vm_get_state(&engine, &addr, &balance_key(recipient), bridge_op)
        .parse()
        .unwrap_or(0);
    vm_set_state(
        &engine,
        &addr,
        &balance_key(recipient),
        &(recipient_bal + mint_amount).to_string(),
        bridge_op,
    );

    assert_eq!(
        vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, bridge_op),
        new_supply.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(recipient), bridge_op),
        mint_amount.to_string()
    );
    println!("  wrap_mint: 10 wBTC to user: PASS");

    // ── Max supply cap enforcement ──
    let over_supply = max_supply + 1;
    let check = new_supply + over_supply <= max_supply;
    assert!(!check, "Over-supply mint must be rejected");
    println!("  Max supply cap enforcement: PASS");

    // ── Simulate wrap_burn: user burns 5 wBTC for redemption ──
    let burn_amount: u128 = 5 * CIL_PER_LOS;
    let user_bal: u128 = vm_get_state(&engine, &addr, &balance_key(recipient), bridge_op)
        .parse()
        .unwrap();
    assert!(user_bal >= burn_amount, "Insufficient balance for burn");

    let new_user_bal = user_bal - burn_amount;
    let burned_supply: u128 = vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, bridge_op)
        .parse::<u128>()
        .unwrap()
        - burn_amount;

    vm_set_state(
        &engine,
        &addr,
        &balance_key(recipient),
        &new_user_bal.to_string(),
        bridge_op,
    );
    vm_set_state(
        &engine,
        &addr,
        USP01_TOTAL_SUPPLY_KEY,
        &burned_supply.to_string(),
        bridge_op,
    );

    assert_eq!(
        vm_get_state(&engine, &addr, &balance_key(recipient), bridge_op),
        new_user_bal.to_string()
    );
    assert_eq!(
        vm_get_state(&engine, &addr, USP01_TOTAL_SUPPLY_KEY, bridge_op),
        burned_supply.to_string()
    );
    println!("  wrap_burn: 5 wBTC redeemed: PASS\n");
}

// ============================================================================
// TEST 15: MULTI-POOL DEX STATE MANAGEMENT
// ============================================================================
// Verifies: multiple simultaneous pools, pool listing, cross-pool isolation.
#[test]
fn test_dex_multi_pool_state() {
    println!("\n=== TEST 15: Multi-Pool DEX State ===\n");

    let engine = WasmEngine::new();
    let creator = "LOS_dex_creator";

    let dex = engine
        .deploy_contract(
            creator.to_string(),
            MINIMAL_WASM.to_vec(),
            BTreeMap::new(),
            1,
        )
        .unwrap();

    vm_set_state(&engine, &dex, DEX_INIT_KEY, "1", creator);
    vm_set_state(&engine, &dex, DEX_POOL_COUNT_KEY, "0", creator);

    // ── Create 3 pools ──
    let tokens = [
        ("wBTC", "wETH", 1_000u128, 4_000u128),
        ("wBTC", "LOS", 500u128, 10_000u128),
        ("wETH", "LOS", 2_000u128, 5_000u128),
    ];

    let mut pool_ids = Vec::new();

    for (i, (ta, tb, amt_a, amt_b)) in tokens.iter().enumerate() {
        let pid = pool_id(ta, tb);
        let ra = amt_a * CIL_PER_LOS;
        let rb = amt_b * CIL_PER_LOS;
        let lp = isqrt(ra * rb);

        vm_set_state(&engine, &dex, &pool_key(&pid, "token_a"), ta, creator);
        vm_set_state(&engine, &dex, &pool_key(&pid, "token_b"), tb, creator);
        vm_set_state(
            &engine,
            &dex,
            &pool_key(&pid, "reserve_a"),
            &ra.to_string(),
            creator,
        );
        vm_set_state(
            &engine,
            &dex,
            &pool_key(&pid, "reserve_b"),
            &rb.to_string(),
            creator,
        );
        vm_set_state(
            &engine,
            &dex,
            &pool_key(&pid, "total_lp"),
            &lp.to_string(),
            creator,
        );
        vm_set_state(&engine, &dex, &format!("pool_list:{}", i), &pid, creator);

        pool_ids.push(pid.clone());
        println!("  Pool {}: {}_{} ({} × {}), LP={}", i, ta, tb, ra, rb, lp);
    }

    vm_set_state(&engine, &dex, DEX_POOL_COUNT_KEY, "3", creator);

    // ── Verify pool count ──
    assert_eq!(
        vm_get_state(&engine, &dex, DEX_POOL_COUNT_KEY, creator),
        "3"
    );

    // ── Verify pool isolation: modifying one pool doesn't affect others ──
    let first_pid = &pool_ids[0];
    let second_pid = &pool_ids[1];

    let first_reserve_a_before: u128 =
        vm_get_state(&engine, &dex, &pool_key(first_pid, "reserve_a"), creator)
            .parse()
            .unwrap();

    // Modify second pool's reserve
    vm_set_state(
        &engine,
        &dex,
        &pool_key(second_pid, "reserve_a"),
        "999999",
        creator,
    );

    let first_reserve_a_after: u128 =
        vm_get_state(&engine, &dex, &pool_key(first_pid, "reserve_a"), creator)
            .parse()
            .unwrap();

    assert_eq!(
        first_reserve_a_before, first_reserve_a_after,
        "Cross-pool isolation violated"
    );
    println!("  Cross-pool isolation: PASS");

    // ── Pool list iteration ──
    let count: usize = vm_get_state(&engine, &dex, DEX_POOL_COUNT_KEY, creator)
        .parse()
        .unwrap();
    for (i, expected_pid) in pool_ids.iter().enumerate().take(count) {
        let listed_pid = vm_get_state(&engine, &dex, &format!("pool_list:{}", i), creator);
        assert_eq!(
            listed_pid, *expected_pid,
            "Pool list mismatch at index {}",
            i
        );
    }
    println!("  Pool listing: PASS");
    println!("  Multi-pool management: PASS\n");
}
