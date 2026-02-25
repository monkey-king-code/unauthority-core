//! Fuzz target: aBFT consensus message deserialization
//!
//! Feeds arbitrary bytes to ConsensusMessage deserialization.
//! Ensures no panics on malformed consensus messages (network attack surface).
//!
//! Run: cargo +nightly fuzz run fuzz_abft_message

#![no_main]
use libfuzzer_sys::fuzz_target;
use los_consensus::abft::ConsensusMessage;

fuzz_target!(|data: &[u8]| {
    // JSON deserialization — must not panic
    if let Ok(s) = std::str::from_utf8(data) {
        let _: Result<ConsensusMessage, _> = serde_json::from_str(s);
    }

    // Raw bytes — must not panic
    let _: Result<ConsensusMessage, _> = serde_json::from_slice(data);
});
