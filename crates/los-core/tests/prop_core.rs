// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PROPERTY-BASED TESTS — los-core
//
// These tests verify mathematical invariants that MUST hold for ALL possible
// inputs. proptest generates thousands of random inputs per property.
//
// ZERO production code changes — this is a #[cfg(test)] integration test.
// Run: cargo test --release -p los-core --test prop_core
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use los_core::pow_mint::{
    compute_mining_hash, count_leading_zero_bits, verify_mining_hash, MiningState,
};
use los_core::{
    Block, BlockType, Ledger, BASE_FEE_CIL, CIL_PER_LOS, MIN_POW_DIFFICULTY_BITS, TOTAL_SUPPLY_CIL,
};
use proptest::prelude::*;

// ─────────────────────────────────────────────────────────────────
// BLOCK PROPERTIES
// ─────────────────────────────────────────────────────────────────

fn arb_block_type() -> impl Strategy<Value = BlockType> {
    prop_oneof![
        Just(BlockType::Send),
        Just(BlockType::Receive),
        Just(BlockType::Change),
        Just(BlockType::Mint),
        Just(BlockType::Slash),
        Just(BlockType::ContractDeploy),
        Just(BlockType::ContractCall),
    ]
}

fn arb_block() -> impl Strategy<Value = Block> {
    (
        "LOS[A-Za-z0-9]{20,40}", // account
        "[0-9a-f]{64}",          // previous
        arb_block_type(),
        0u128..=TOTAL_SUPPLY_CIL,            // amount
        ".*",                                // link
        "[0-9a-f]{0,128}",                   // signature
        "[0-9a-f]{0,128}",                   // public_key
        any::<u64>(),                        // work
        1_700_000_000u64..=2_000_000_000u64, // timestamp
        0u128..=BASE_FEE_CIL * 1000,         // fee
    )
        .prop_map(
            |(
                account,
                previous,
                block_type,
                amount,
                link,
                signature,
                public_key,
                work,
                timestamp,
                fee,
            )| {
                Block {
                    account,
                    previous,
                    block_type,
                    amount,
                    link,
                    signature,
                    public_key,
                    work,
                    timestamp,
                    fee,
                }
            },
        )
}

proptest! {
    /// PROPERTY: signing_hash is deterministic — same block always yields same hash
    #[test]
    fn prop_signing_hash_deterministic(block in arb_block()) {
        let h1 = block.signing_hash();
        let h2 = block.signing_hash();
        prop_assert_eq!(h1, h2, "signing_hash must be deterministic");
    }

    /// PROPERTY: calculate_hash is deterministic
    #[test]
    fn prop_calculate_hash_deterministic(block in arb_block()) {
        let h1 = block.calculate_hash();
        let h2 = block.calculate_hash();
        prop_assert_eq!(h1, h2, "calculate_hash must be deterministic");
    }

    /// PROPERTY: signing_hash ≠ calculate_hash (signature is included in full hash)
    #[test]
    fn prop_signing_hash_differs_from_full(block in arb_block()) {
        let sh = block.signing_hash();
        let ch = block.calculate_hash();
        // They SHOULD differ because calculate_hash includes the signature
        // (unless signature happens to not affect the hash — but SHA3 ensures it does)
        prop_assert_ne!(sh, ch, "signing_hash and calculate_hash should differ");
    }

    /// PROPERTY: signing_hash output is always 64 hex chars (SHA3-256 = 32 bytes = 64 hex)
    #[test]
    fn prop_signing_hash_length(block in arb_block()) {
        let hash = block.signing_hash();
        prop_assert_eq!(hash.len(), 64, "SHA3-256 hash must be 64 hex chars");
        prop_assert!(hash.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash must be valid hex");
    }

    /// PROPERTY: Different amounts produce different signing hashes
    #[test]
    fn prop_different_amounts_different_hash(
        amount1 in 0u128..=1_000_000u128,
        amount2 in 1_000_001u128..=2_000_000u128,
    ) {
        let block1 = Block {
            account: "LOStest1".to_string(),
            previous: "0".repeat(64),
            block_type: BlockType::Send,
            amount: amount1,
            link: "LOStest2".to_string(),
            signature: String::new(),
            public_key: String::new(),
            work: 0,
            timestamp: 1_700_000_000,
            fee: 0,
        };
        let block2 = Block { amount: amount2, ..block1.clone() };
        prop_assert_ne!(block1.signing_hash(), block2.signing_hash());
    }
}

// ─────────────────────────────────────────────────────────────────
// MINING HASH PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: compute_mining_hash is deterministic
    #[test]
    fn prop_mining_hash_deterministic(
        address in "LOS[A-Za-z0-9]{20,40}",
        epoch in 0u64..=100_000,
        nonce in any::<u64>(),
    ) {
        let h1 = compute_mining_hash(&address, epoch, nonce);
        let h2 = compute_mining_hash(&address, epoch, nonce);
        prop_assert_eq!(h1, h2);
    }

    /// PROPERTY: leading zero bit count is always ≤256 and consistent
    #[test]
    fn prop_leading_zeros_bounded(data in proptest::collection::vec(any::<u8>(), 1..64)) {
        let bits = count_leading_zero_bits(&data);
        prop_assert!(bits <= data.len() as u32 * 8,
            "Cannot have more zero bits than total bits");
    }

    /// PROPERTY: all-zero bytes have exactly len*8 leading zero bits
    #[test]
    fn prop_all_zeros(len in 1usize..=64) {
        let data = vec![0u8; len];
        let bits = count_leading_zero_bits(&data);
        prop_assert_eq!(bits, (len as u32) * 8);
    }

    /// PROPERTY: verify_mining_hash is consistent with manual check
    #[test]
    fn prop_verify_consistent(
        address in "LOS[A-Za-z0-9]{10,20}",
        epoch in 0u64..=1000,
        nonce in any::<u64>(),
        difficulty in 0u32..=256,
    ) {
        let hash = compute_mining_hash(&address, epoch, nonce);
        let zeros = count_leading_zero_bits(&hash);
        let verified = verify_mining_hash(&address, epoch, nonce, difficulty);
        prop_assert_eq!(verified, zeros >= difficulty,
            "verify_mining_hash must agree with manual zero-bit count");
    }

    /// PROPERTY: Different addresses produce different hashes (collision resistance)
    #[test]
    fn prop_address_binding(
        addr1 in "LOSA[A-Za-z0-9]{10,20}",
        addr2 in "LOSB[A-Za-z0-9]{10,20}",
        epoch in 0u64..=1000,
        nonce in any::<u64>(),
    ) {
        let h1 = compute_mining_hash(&addr1, epoch, nonce);
        let h2 = compute_mining_hash(&addr2, epoch, nonce);
        prop_assert_ne!(h1, h2, "Different addresses should produce different hashes");
    }
}

// ─────────────────────────────────────────────────────────────────
// MINING STATE PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: Epoch halving reward monotonically decreases
    #[test]
    fn prop_halving_monotonic(epoch in 0u64..=100_000) {
        let r1 = MiningState::epoch_reward_cil(epoch);
        let r2 = MiningState::epoch_reward_cil(epoch + 1);
        prop_assert!(r2 <= r1, "Reward must never increase between epochs: {} > {}", r2, r1);
    }

    /// PROPERTY: Epoch reward is always bounded by initial reward
    #[test]
    fn prop_reward_bounded(epoch in any::<u64>()) {
        let reward = MiningState::epoch_reward_cil(epoch);
        prop_assert!(reward <= los_core::pow_mint::MINING_REWARD_PER_EPOCH_CIL,
            "Reward cannot exceed initial rate");
    }

    /// PROPERTY: Difficulty stays within bounds after advance_epoch
    #[test]
    fn prop_difficulty_bounded(
        initial_diff in 16u32..=40,
        num_miners in 0usize..=100,
        genesis_ts in 1_700_000_000u64..=1_800_000_000u64,
    ) {
        let mut state = MiningState::new(genesis_ts);
        state.difficulty_bits = initial_diff;

        // Add miners to current epoch
        for i in 0..num_miners.min(50) {
            state.current_epoch_miners.insert(format!("LOSminer{}", i));
        }

        state.advance_epoch(state.current_epoch + 1);

        prop_assert!(state.difficulty_bits >= los_core::pow_mint::MIN_MINING_DIFFICULTY_BITS,
            "Difficulty below minimum: {}", state.difficulty_bits);
        prop_assert!(state.difficulty_bits <= los_core::pow_mint::MAX_MINING_DIFFICULTY_BITS,
            "Difficulty above maximum: {}", state.difficulty_bits);
    }
}

// ─────────────────────────────────────────────────────────────────
// LEDGER INVARIANTS
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: state_root is deterministic (same accounts → same hash)
    #[test]
    fn prop_state_root_deterministic(
        balances in proptest::collection::vec(
            (1u128..=1_000_000 * CIL_PER_LOS),
            1..10
        ),
    ) {
        let mut ledger = Ledger::new();
        for (i, bal) in balances.iter().enumerate() {
            ledger.accounts.insert(
                format!("LOSaddr{:04}", i),
                los_core::AccountState {
                    head: String::new(),
                    balance: *bal,
                    block_count: 0,
                    is_validator: false,
                },
            );
        }
        let root1 = ledger.compute_state_root();
        let root2 = ledger.compute_state_root();
        prop_assert_eq!(root1, root2, "state_root must be deterministic");
    }

    /// PROPERTY: state_root changes when any balance changes
    #[test]
    fn prop_state_root_sensitive(
        balance1 in 1u128..=1_000_000u128,
        balance2 in 1_000_001u128..=2_000_000u128,
    ) {
        let mut ledger1 = Ledger::new();
        ledger1.accounts.insert(
            "LOStest1".to_string(),
            los_core::AccountState {
                head: String::new(),
                balance: balance1,
                block_count: 0,
                is_validator: false,
            },
        );

        let mut ledger2 = Ledger::new();
        ledger2.accounts.insert(
            "LOStest1".to_string(),
            los_core::AccountState {
                head: String::new(),
                balance: balance2,
                block_count: 0,
                is_validator: false,
            },
        );

        prop_assert_ne!(
            ledger1.compute_state_root(),
            ledger2.compute_state_root(),
            "Different balances must produce different state roots"
        );
    }

    /// PROPERTY: Empty ledger always has the same state root
    #[test]
    fn prop_empty_ledger_root_constant(_dummy in 0u8..=255) {
        let l1 = Ledger::new();
        let l2 = Ledger::new();
        prop_assert_eq!(l1.compute_state_root(), l2.compute_state_root());
    }
}
