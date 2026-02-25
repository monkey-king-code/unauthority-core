//! Fuzz target: Block signing hash determinism and crash-resistance
//!
//! Constructs Blocks from structured fuzz input and verifies:
//! 1. signing_hash() never panics
//! 2. signing_hash() is deterministic (same input → same output)
//! 3. calculate_hash() never panics
//!
//! Run: cargo +nightly fuzz run fuzz_block_signing_hash

#![no_main]
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use los_core::{Block, BlockType};

#[derive(Arbitrary, Debug)]
struct FuzzBlock {
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

impl From<FuzzBlock> for Block {
    fn from(fb: FuzzBlock) -> Self {
        let block_type = match fb.block_type_idx % 7 {
            0 => BlockType::Send,
            1 => BlockType::Receive,
            2 => BlockType::Change,
            3 => BlockType::Mint,
            4 => BlockType::Slash,
            5 => BlockType::ContractDeploy,
            _ => BlockType::ContractCall,
        };
        Block {
            account: fb.account,
            previous: fb.previous,
            block_type,
            amount: fb.amount,
            link: fb.link,
            signature: fb.signature,
            public_key: fb.public_key,
            work: fb.work,
            timestamp: fb.timestamp,
            fee: fb.fee,
        }
    }
}

fuzz_target!(|fb: FuzzBlock| {
    let block: Block = fb.into();

    // Must not panic
    let hash1 = block.signing_hash();
    let hash2 = block.signing_hash();

    // Determinism: same block → same hash
    assert_eq!(hash1, hash2, "signing_hash must be deterministic");

    // calculate_hash must not panic
    let full1 = block.calculate_hash();
    let full2 = block.calculate_hash();
    assert_eq!(full1, full2, "calculate_hash must be deterministic");

    // verify_signature must not panic (will return false for random data)
    let _ = block.verify_signature();

    // verify_pow must not panic
    let _ = block.verify_pow();
});
