// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PROPERTY-BASED TESTS — los-crypto
//
// Verifies cryptographic invariants:
// - Key generation determinism from seed
// - Address derivation consistency
// - Sign/verify round-trip integrity
// - Address validation accepts valid, rejects invalid
//
// ZERO production code changes — integration test file only.
// Run: cargo test --release -p los-crypto --test prop_crypto
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use los_crypto::{
    generate_keypair, generate_keypair_from_seed, public_key_to_address,
    sign_message, validate_address, verify_signature,
};
use proptest::prelude::*;

// ─────────────────────────────────────────────────────────────────
// ADDRESS PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: Keypair → address → validate always succeeds
    #[test]
    fn prop_generated_address_valid(_dummy in 0u8..=3) {
        // Generate limited times (Dilithium5 keygen is slow)
        let kp = generate_keypair();
        let addr = public_key_to_address(&kp.public_key);
        prop_assert!(validate_address(&addr),
            "Generated address must pass validation: {}", addr);
    }

    /// PROPERTY: Address always starts with "LOS"
    #[test]
    fn prop_address_prefix(_dummy in 0u8..=3) {
        let kp = generate_keypair();
        let addr = public_key_to_address(&kp.public_key);
        prop_assert!(addr.starts_with("LOS"),
            "Address must start with LOS: {}", addr);
    }

    /// PROPERTY: Invalid strings are rejected
    #[test]
    fn prop_garbage_address_rejected(garbage in "[^L][A-Za-z0-9]{0,50}") {
        prop_assert!(!validate_address(&garbage),
            "Random string should fail validation: {}", garbage);
    }

    /// PROPERTY: Empty and short strings are rejected
    #[test]
    fn prop_short_address_rejected(len in 0usize..=5) {
        let s: String = (0..len).map(|_| 'L').collect();
        prop_assert!(!validate_address(&s));
    }

    /// PROPERTY: Corrupted address (flipped char) is rejected
    #[test]
    fn prop_corrupted_address_rejected(
        _dummy in 0u8..=1,
        flip_pos in 3usize..=35,
    ) {
        let kp = generate_keypair();
        let addr = public_key_to_address(&kp.public_key);
        if flip_pos < addr.len() {
            let mut chars: Vec<char> = addr.chars().collect();
            // Flip a character
            chars[flip_pos] = if chars[flip_pos] == 'a' { 'b' } else { 'a' };
            let corrupted: String = chars.into_iter().collect();
            // Corrupted address SHOULD fail (checksum catches it)
            // Note: very rare false-positive possible (1/2^32) — acceptable for property test
            prop_assert!(!validate_address(&corrupted),
                "Corrupted address should fail: {} → {}", addr, corrupted);
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// DETERMINISTIC KEYGEN PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: Same seed always produces same keypair and address
    #[test]
    fn prop_deterministic_keygen(seed in proptest::collection::vec(any::<u8>(), 32..=64)) {
        let kp1 = generate_keypair_from_seed(&seed);
        let kp2 = generate_keypair_from_seed(&seed);

        prop_assert_eq!(&kp1.public_key, &kp2.public_key,
            "Same seed must produce same public key");

        let addr1 = public_key_to_address(&kp1.public_key);
        let addr2 = public_key_to_address(&kp2.public_key);
        prop_assert_eq!(addr1, addr2, "Same seed must produce same address");
    }

    /// PROPERTY: Different seeds produce different addresses
    #[test]
    fn prop_different_seeds_different_addresses(
        seed1 in proptest::collection::vec(0u8..=127, 32..=32),
        seed2 in proptest::collection::vec(128u8..=255, 32..=32),
    ) {
        let kp1 = generate_keypair_from_seed(&seed1);
        let kp2 = generate_keypair_from_seed(&seed2);
        let addr1 = public_key_to_address(&kp1.public_key);
        let addr2 = public_key_to_address(&kp2.public_key);
        prop_assert_ne!(addr1, addr2, "Different seeds should produce different addresses");
    }
}

// ─────────────────────────────────────────────────────────────────
// SIGN / VERIFY ROUND-TRIP PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: sign then verify always succeeds with correct key
    #[test]
    fn prop_sign_verify_roundtrip(
        message in proptest::collection::vec(any::<u8>(), 0..=1024),
    ) {
        let kp = generate_keypair();
        let sig = sign_message(&message, &kp.secret_key)
            .expect("Signing must succeed with valid key");

        let valid = verify_signature(&message, &sig, &kp.public_key);
        prop_assert!(valid, "Signature must verify with correct key");
    }

    /// PROPERTY: Verification fails with wrong public key
    #[test]
    fn prop_wrong_key_fails(
        message in proptest::collection::vec(any::<u8>(), 1..=256),
    ) {
        let kp1 = generate_keypair();
        let kp2 = generate_keypair();
        let sig = sign_message(&message, &kp1.secret_key)
            .expect("Signing must succeed");

        let valid = verify_signature(&message, &sig, &kp2.public_key);
        prop_assert!(!valid, "Signature must NOT verify with wrong key");
    }

    /// PROPERTY: Verification fails with tampered message
    #[test]
    fn prop_tampered_message_fails(
        message in proptest::collection::vec(any::<u8>(), 1..=256),
    ) {
        let kp = generate_keypair();
        let sig = sign_message(&message, &kp.secret_key)
            .expect("Signing must succeed");

        let mut tampered = message.clone();
        tampered[0] = tampered[0].wrapping_add(1);

        let valid = verify_signature(&tampered, &sig, &kp.public_key);
        prop_assert!(!valid, "Tampered message must fail verification");
    }

    /// PROPERTY: Empty signature always fails
    #[test]
    fn prop_empty_sig_fails(
        message in proptest::collection::vec(any::<u8>(), 0..=64),
    ) {
        let kp = generate_keypair();
        let valid = verify_signature(&message, &[], &kp.public_key);
        prop_assert!(!valid, "Empty signature must fail");
    }

    /// PROPERTY: Garbage signature always fails
    #[test]
    fn prop_garbage_sig_fails(
        message in proptest::collection::vec(any::<u8>(), 0..=64),
        garbage in proptest::collection::vec(any::<u8>(), 1..=128),
    ) {
        let kp = generate_keypair();
        let valid = verify_signature(&message, &garbage, &kp.public_key);
        prop_assert!(!valid, "Garbage signature must fail");
    }
}
