//! Patched pqcrypto-internals with deterministic seeded randombytes support.
//!
//! This is a modified version of pqcrypto-internals 0.2.11 that adds the ability
//! to use a thread-local seeded CSPRNG (ChaCha20) instead of OS entropy.
//!
//! When `set_seeded_rng(seed)` is called, subsequent calls to
//! `PQCRYPTO_RUST_randombytes()` on the same thread will use the seeded ChaCha20
//! CSPRNG. This enables deterministic Dilithium5 keypair generation from a BIP39 seed.

extern crate alloc;

use core::slice;

// Re-export seeded RNG functionality when the "seeded" feature is enabled
#[cfg(feature = "seeded")]
mod seeded {
    use rand::RngCore;
    use rand::SeedableRng;
    use rand_chacha::ChaCha20Rng;
    use std::cell::RefCell;

    thread_local! {
        pub(crate) static SEEDED_RNG: RefCell<Option<ChaCha20Rng>> = RefCell::new(None);
    }

    /// Activate deterministic mode: all `PQCRYPTO_RUST_randombytes()` calls
    /// on this thread will use a ChaCha20 CSPRNG seeded with the given 32-byte seed.
    pub fn set_seeded_rng(seed: [u8; 32]) {
        SEEDED_RNG.with(|cell| {
            cell.replace(Some(ChaCha20Rng::from_seed(seed)));
        });
    }

    /// Deactivate deterministic mode: revert to OS-RNG for subsequent calls.
    pub fn clear_seeded_rng() {
        SEEDED_RNG.with(|cell| {
            cell.replace(None);
        });
    }

    /// Try to fill buffer from seeded RNG. Returns true if seeded mode is active.
    pub fn try_fill_seeded(buf: &mut [u8]) -> bool {
        SEEDED_RNG.with(|cell| {
            let mut opt = cell.borrow_mut();
            if let Some(rng) = opt.as_mut() {
                rng.fill_bytes(buf);
                true
            } else {
                false
            }
        })
    }
}

#[cfg(feature = "seeded")]
pub use seeded::{clear_seeded_rng, set_seeded_rng};

/// Get random bytes; exposed for PQClean implementations.
///
/// If a seeded ChaCha20 CSPRNG is active (via `set_seeded_rng()`), uses it
/// for deterministic output. Otherwise, falls back to OS entropy via `getrandom`.
///
/// # Safety
/// Assumes `buf` is a valid pointer to `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn PQCRYPTO_RUST_randombytes(
    buf: *mut u8,
    len: libc::size_t,
) -> libc::c_int {
    let buf = slice::from_raw_parts_mut(buf, len);

    // Check seeded mode first (only when "seeded" feature is enabled)
    #[cfg(feature = "seeded")]
    {
        if seeded::try_fill_seeded(buf) {
            return 0;
        }
    }

    // Default: OS entropy
    getrandom::fill(buf).expect("RNG Failed");
    0
}
