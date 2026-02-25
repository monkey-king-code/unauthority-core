//! Fuzz target: Mining hash computation and verification
//!
//! Verifies:
//! 1. compute_mining_hash() never panics on arbitrary input
//! 2. verify_mining_hash() never panics
//! 3. Hash is deterministic
//! 4. Leading zero bit count is consistent
//!
//! Run: cargo +nightly fuzz run fuzz_mining_hash

#![no_main]
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use los_core::pow_mint;

#[derive(Arbitrary, Debug)]
struct FuzzMiningInput {
    address: String,
    epoch: u64,
    nonce: u64,
    difficulty_bits: u32,
}

fuzz_target!(|input: FuzzMiningInput| {
    // compute_mining_hash must not panic
    let hash1 = pow_mint::compute_mining_hash(&input.address, input.epoch, input.nonce);
    let hash2 = pow_mint::compute_mining_hash(&input.address, input.epoch, input.nonce);

    // Determinism
    assert_eq!(hash1, hash2, "compute_mining_hash must be deterministic");

    // count_leading_zero_bits must not panic and must be ≤256
    let bits = pow_mint::count_leading_zero_bits(&hash1);
    assert!(bits <= 256, "leading zero bits cannot exceed 256");

    // verify_mining_hash must not panic
    // Clamp difficulty to valid range to avoid meaningless tests
    let difficulty = input.difficulty_bits.min(256);
    let result = pow_mint::verify_mining_hash(
        &input.address,
        input.epoch,
        input.nonce,
        difficulty,
    );

    // Consistency: if bits >= difficulty, verify should return true
    if bits >= difficulty {
        assert!(result, "verify should return true when hash meets difficulty");
    }
});
