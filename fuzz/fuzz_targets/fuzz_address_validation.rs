//! Fuzz target: LOS address validation
//!
//! Feeds arbitrary strings to validate_address() to ensure:
//! 1. No panics on any input
//! 2. Valid addresses round-trip correctly
//!
//! Run: cargo +nightly fuzz run fuzz_address_validation -- -max_len=256

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // validate_address must never panic, even on garbage input
        let _ = los_crypto::validate_address(s);
    }

    // Also test public_key_to_address with arbitrary bytes
    // Must not panic even with wrong-length keys
    if !data.is_empty() {
        let addr = los_crypto::public_key_to_address(data);

        // If we got an address, it MUST be valid
        assert!(
            los_crypto::validate_address(&addr),
            "Generated address must pass validation: {}",
            addr
        );
    }
});
