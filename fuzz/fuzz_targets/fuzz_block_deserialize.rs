//! Fuzz target: Block JSON deserialization
//!
//! Feeds arbitrary bytes to serde_json to detect panics, stack overflows,
//! or unexpected behavior in Block deserialization.
//!
//! Run: cargo +nightly fuzz run fuzz_block_deserialize -- -max_len=4096

#![no_main]
use libfuzzer_sys::fuzz_target;
use los_core::Block;

fuzz_target!(|data: &[u8]| {
    // Attempt JSON deserialization — must not panic
    if let Ok(s) = std::str::from_utf8(data) {
        let _: Result<Block, _> = serde_json::from_str(s);
    }

    // Also test from raw bytes (content-type: application/octet-stream attack)
    let _: Result<Block, _> = serde_json::from_slice(data);
});
