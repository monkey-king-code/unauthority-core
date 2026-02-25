//! Fuzz target: Ledger process_block robustness
//!
//! Feeds structurally-valid-but-random blocks to Ledger::process_block().
//! Verifies the ledger never panics and always returns Ok/Err gracefully.
//!
//! Run: cargo +nightly fuzz run fuzz_ledger_process_block

#![no_main]
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use los_core::{Block, BlockType, Ledger};

#[derive(Arbitrary, Debug)]
struct FuzzLedgerInput {
    // Pre-seed some accounts
    seed_accounts: Vec<(String, u128)>,
    // Block to process
    account: String,
    previous: String,
    block_type_idx: u8,
    amount: u128,
    link: String,
    signature: String,
    public_key: String,
    work: u64,
    timestamp: u64,
    fee: u128,
}

fuzz_target!(|input: FuzzLedgerInput| {
    let mut ledger = Ledger::new();

    // Seed up to 8 accounts (prevent OOM from huge vectors)
    for (addr, balance) in input.seed_accounts.iter().take(8) {
        if !addr.is_empty() {
            ledger.accounts.insert(
                addr.clone(),
                los_core::AccountState {
                    head: String::new(),
                    balance: *balance,
                    block_count: 0,
                    is_validator: false,
                },
            );
        }
    }

    let block_type = match input.block_type_idx % 7 {
        0 => BlockType::Send,
        1 => BlockType::Receive,
        2 => BlockType::Change,
        3 => BlockType::Mint,
        4 => BlockType::Slash,
        5 => BlockType::ContractDeploy,
        _ => BlockType::ContractCall,
    };

    let block = Block {
        account: input.account,
        previous: input.previous,
        block_type,
        amount: input.amount,
        link: input.link,
        signature: input.signature,
        public_key: input.public_key,
        work: input.work,
        timestamp: input.timestamp,
        fee: input.fee,
    };

    // process_block must NEVER panic — only Ok() or Err()
    let _ = ledger.process_block(&block);
});
