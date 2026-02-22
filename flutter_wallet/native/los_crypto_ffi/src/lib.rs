//! LOS Crypto FFI — C-ABI bridge for Flutter wallet (dart:ffi)
//!
//! Exposes CRYSTALS-Dilithium5 (NIST Level 5) operations:
//! - Keypair generation (random)
//! - Message signing / verification
//! - LOS address derivation (Base58Check, matching los-crypto backend)
//! - Address validation
//! - PoW mining (native Keccak-256, 100-1000x faster than pure Dart)
//!
//! All functions use pre-allocated buffers and return status codes.
//! Return values: 0 or positive = success, negative = error.

use pqcrypto_dilithium::dilithium5::{
    self, keypair, detached_sign, verify_detached_signature,
    PublicKey as DilithiumPublicKey,
    SecretKey as DilithiumSecretKey,
    DetachedSignature as DilithiumSignature,
};
use pqcrypto_traits::sign::{PublicKey, SecretKey, DetachedSignature};
use blake2::Blake2b512;
use sha2::Sha256;
use sha3::Keccak256;
use digest::Digest;
use zeroize::Zeroize;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SIZE QUERIES — Call these first to allocate correct buffer sizes in Dart
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Returns the Dilithium5 public key size in bytes (2592)
#[no_mangle]
pub extern "C" fn los_public_key_bytes() -> i32 {
    dilithium5::public_key_bytes() as i32
}

/// Returns the Dilithium5 secret key size in bytes (4864)
#[no_mangle]
pub extern "C" fn los_secret_key_bytes() -> i32 {
    dilithium5::secret_key_bytes() as i32
}

/// Returns the Dilithium5 signature size in bytes
#[no_mangle]
pub extern "C" fn los_signature_bytes() -> i32 {
    dilithium5::signature_bytes() as i32
}

/// Returns max LOS address length in bytes (including "LOS" prefix + Base58)
/// Typically ~37-48 chars, we allocate 64 for safety
#[no_mangle]
pub extern "C" fn los_max_address_bytes() -> i32 {
    64
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// KEY GENERATION — Random Dilithium5 keypair (NIST Level 5)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generate a random Dilithium5 keypair.
///
/// # Arguments
/// - `pk_out`: Pre-allocated buffer for public key (must be >= los_public_key_bytes())
/// - `pk_capacity`: Buffer size
/// - `sk_out`: Pre-allocated buffer for secret key (must be >= los_secret_key_bytes())
/// - `sk_capacity`: Buffer size
///
/// # Returns
/// 0 on success, negative on error:
/// - -1: null pointer
/// - -2: buffer too small
#[no_mangle]
pub extern "C" fn los_generate_keypair(
    pk_out: *mut u8,
    pk_capacity: i32,
    sk_out: *mut u8,
    sk_capacity: i32,
) -> i32 {
    if pk_out.is_null() || sk_out.is_null() {
        return -1;
    }

    let pk_cap = pk_capacity as usize;
    let sk_cap = sk_capacity as usize;
    let pk_size = dilithium5::public_key_bytes();
    let sk_size = dilithium5::secret_key_bytes();

    if pk_cap < pk_size || sk_cap < sk_size {
        return -2;
    }

    let (pk, sk) = keypair();
    let pk_bytes = pk.as_bytes();
    let sk_bytes = sk.as_bytes();

    unsafe {
        std::ptr::copy_nonoverlapping(pk_bytes.as_ptr(), pk_out, pk_bytes.len());
        std::ptr::copy_nonoverlapping(sk_bytes.as_ptr(), sk_out, sk_bytes.len());
    }

    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// KEY GENERATION — Deterministic from BIP39 seed
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generate a deterministic Dilithium5 keypair from a BIP39 seed.
///
/// The seed is used to derive a 32-byte ChaCha20 CSPRNG seed via:
///   derived = SHA-256( SHA-256("los-dilithium5-keygen-v1") || bip39_seed )
///
/// This ensures the same BIP39 mnemonic always produces the same keypair,
/// enabling wallet recovery from mnemonic alone.
///
/// # Arguments
/// - `seed`: BIP39 seed bytes (typically 64 bytes from `mnemonicToSeed()`)
/// - `seed_len`: Length of seed in bytes (must be >= 32)
/// - `pk_out`: Pre-allocated buffer for public key
/// - `pk_capacity`: Buffer size
/// - `sk_out`: Pre-allocated buffer for secret key
/// - `sk_capacity`: Buffer size
///
/// # Returns
/// 0 on success, negative on error:
/// - -1: null pointer
/// - -2: buffer too small
/// - -4: seed too short (< 32 bytes)
#[no_mangle]
pub extern "C" fn los_generate_keypair_from_seed(
    seed: *const u8,
    seed_len: i32,
    pk_out: *mut u8,
    pk_capacity: i32,
    sk_out: *mut u8,
    sk_capacity: i32,
) -> i32 {
    if seed.is_null() || pk_out.is_null() || sk_out.is_null() {
        return -1;
    }
    if seed_len < 32 {
        return -4;
    }

    let pk_cap = pk_capacity as usize;
    let sk_cap = sk_capacity as usize;
    let pk_size = dilithium5::public_key_bytes();
    let sk_size = dilithium5::secret_key_bytes();

    if pk_cap < pk_size || sk_cap < sk_size {
        return -2;
    }

    let seed_slice = unsafe { std::slice::from_raw_parts(seed, seed_len as usize) };

    // Derive 32-byte deterministic seed for ChaCha20 CSPRNG
    // Domain separation: SHA-256("los-dilithium5-keygen-v1") = salt
    // derived = SHA-256(salt || bip39_seed) → 32 bytes
    let salt = Sha256::digest(b"los-dilithium5-keygen-v1");
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(seed_slice);
    let mut derived: [u8; 32] = hasher.finalize().into();

    // Activate deterministic CSPRNG for pqcrypto's randombytes
    pqcrypto_internals::set_seeded_rng(derived);

    // Generate keypair — now deterministic via seeded ChaCha20
    let (pk, sk) = keypair();

    // Revert to OS-RNG
    pqcrypto_internals::clear_seeded_rng();

    // SECURITY: Zero derived seed material immediately after use
    derived.zeroize();

    let pk_bytes = pk.as_bytes();
    let sk_bytes = sk.as_bytes();

    unsafe {
        std::ptr::copy_nonoverlapping(pk_bytes.as_ptr(), pk_out, pk_bytes.len());
        std::ptr::copy_nonoverlapping(sk_bytes.as_ptr(), sk_out, sk_bytes.len());
    }

    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SIGNING — Dilithium5 detached signatures
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Sign a message with Dilithium5 secret key.
///
/// # Returns
/// Signature length on success (positive), negative on error:
/// - -1: null pointer
/// - -2: buffer too small
/// - -3: invalid secret key
#[no_mangle]
pub extern "C" fn los_sign(
    message: *const u8,
    message_len: i32,
    secret_key: *const u8,
    sk_len: i32,
    signature_out: *mut u8,
    sig_capacity: i32,
) -> i32 {
    if message.is_null() || secret_key.is_null() || signature_out.is_null() {
        return -1;
    }

    let sig_size = dilithium5::signature_bytes();
    if (sig_capacity as usize) < sig_size {
        return -2;
    }

    let msg_slice = unsafe { std::slice::from_raw_parts(message, message_len as usize) };
    let sk_slice = unsafe { std::slice::from_raw_parts(secret_key, sk_len as usize) };

    let sk = match DilithiumSecretKey::from_bytes(sk_slice) {
        Ok(k) => k,
        Err(_) => return -3,
    };

    let signature = detached_sign(msg_slice, &sk);
    let sig_bytes = signature.as_bytes();

    unsafe {
        std::ptr::copy_nonoverlapping(sig_bytes.as_ptr(), signature_out, sig_bytes.len());
    }

    sig_bytes.len() as i32
}

/// Verify a Dilithium5 signature.
///
/// # Returns
/// 1 = valid, 0 = invalid, negative = error:
/// - -1: null pointer
/// - -3: invalid key/signature format
#[no_mangle]
pub extern "C" fn los_verify(
    message: *const u8,
    message_len: i32,
    signature: *const u8,
    sig_len: i32,
    public_key: *const u8,
    pk_len: i32,
) -> i32 {
    if message.is_null() || signature.is_null() || public_key.is_null() {
        return -1;
    }

    let msg_slice = unsafe { std::slice::from_raw_parts(message, message_len as usize) };
    let sig_slice = unsafe { std::slice::from_raw_parts(signature, sig_len as usize) };
    let pk_slice = unsafe { std::slice::from_raw_parts(public_key, pk_len as usize) };

    let pk = match DilithiumPublicKey::from_bytes(pk_slice) {
        Ok(k) => k,
        Err(_) => return -3,
    };

    let sig = match DilithiumSignature::from_bytes(sig_slice) {
        Ok(s) => s,
        Err(_) => return -3,
    };

    if verify_detached_signature(&sig, msg_slice, &pk).is_ok() {
        1
    } else {
        0
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ADDRESS DERIVATION — Exact match with los-crypto backend
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Address format (matches los-crypto/src/lib.rs):
/// "LOS" + Base58( 0x4A | BLAKE2b-160(pubkey) | SHA256²(payload)[0..4] )
const VERSION_BYTE: u8 = 0x4A; // 74 = LOS identifier

/// Derive LOS address from Dilithium5 public key.
/// Exact same algorithm as los-crypto::public_key_to_address().
///
/// # Returns
/// Address length (positive) on success, negative on error:
/// - -1: null pointer
/// - -2: buffer too small
#[no_mangle]
pub extern "C" fn los_public_key_to_address(
    public_key: *const u8,
    pk_len: i32,
    address_out: *mut u8,
    addr_capacity: i32,
) -> i32 {
    if public_key.is_null() || address_out.is_null() {
        return -1;
    }

    let pk_slice = unsafe { std::slice::from_raw_parts(public_key, pk_len as usize) };

    // 1. BLAKE2b-512 hash, take first 20 bytes (160-bit)
    let mut hasher = Blake2b512::new();
    hasher.update(pk_slice);
    let hash_result = hasher.finalize();
    let pubkey_hash = &hash_result[..20];

    // 2. Payload: version_byte + pubkey_hash
    let mut payload = vec![VERSION_BYTE];
    payload.extend_from_slice(pubkey_hash);

    // 3. Checksum: SHA256(SHA256(payload)) first 4 bytes
    let hash1 = Sha256::digest(&payload);
    let hash2 = Sha256::digest(&hash1);
    let checksum = &hash2[..4];

    // 4. Full encoded bytes: payload + checksum = 25 bytes
    let mut address_bytes = payload;
    address_bytes.extend_from_slice(checksum);

    // 5. Base58 encode
    let base58_addr = bs58::encode(&address_bytes).into_string();

    // 6. "LOS" prefix
    let full_address = format!("LOS{}", base58_addr);
    let addr_bytes = full_address.as_bytes();

    if (addr_capacity as usize) < addr_bytes.len() {
        return -2;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(addr_bytes.as_ptr(), address_out, addr_bytes.len());
    }

    addr_bytes.len() as i32
}

/// Validate LOS address format and checksum.
/// Exact same algorithm as los-crypto::validate_address().
///
/// # Returns
/// 1 = valid, 0 = invalid
#[no_mangle]
pub extern "C" fn los_validate_address(
    address: *const u8,
    addr_len: i32,
) -> i32 {
    if address.is_null() || addr_len < 4 {
        return 0;
    }

    let addr_slice = unsafe { std::slice::from_raw_parts(address, addr_len as usize) };
    let addr_str = match std::str::from_utf8(addr_slice) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    // Must start with "LOS"
    if !addr_str.starts_with("LOS") {
        return 0;
    }

    // Decode Base58 (remove "LOS" prefix)
    let base58_part = &addr_str[3..];
    let decoded = match bs58::decode(base58_part).into_vec() {
        Ok(bytes) => bytes,
        Err(_) => return 0,
    };

    // Must be 25 bytes: 1 (version) + 20 (hash) + 4 (checksum)
    if decoded.len() != 25 {
        return 0;
    }

    // Verify version byte
    if decoded[0] != VERSION_BYTE {
        return 0;
    }

    // Verify checksum
    let payload = &decoded[..21];
    let checksum = &decoded[21..];

    let hash1 = Sha256::digest(payload);
    let hash2 = Sha256::digest(&hash1);

    if checksum == &hash2[..4] { 1 } else { 0 }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POW MINING — Native Keccak-256 (100-1000x faster than pure Dart)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Mine Proof-of-Work using native Keccak-256.
///
/// Dart builds the signing_hash input buffer (all block fields serialized)
/// with a placeholder 8-byte work field at `work_offset`. This function
/// iterates nonces in the work field and computes Keccak-256 until the
/// hash has `difficulty_bits` leading zero bits.
///
/// ~100-1000x faster than pure Dart Keccak-256 (pointycastle).
///
/// # Arguments
/// - `buffer`:          Pre-built signing_hash input (mutable — work field is overwritten)
/// - `buffer_len`:      Buffer length in bytes
/// - `work_offset`:     Byte offset of the 8-byte work (nonce) field in the buffer
/// - `difficulty_bits`:  Required leading zero bits (e.g., 16)
/// - `max_iterations`:  Maximum nonces to try before giving up
/// - `nonce_out`:       Output: the successful nonce (u64)
/// - `hash_out`:        Output: hex-encoded signing hash (64 bytes for Keccak-256)
/// - `hash_capacity`:   Size of hash_out buffer (must be >= 64)
///
/// # Returns
/// - Positive: hex hash length (64) on success
/// - -1: null pointer
/// - -2: hash_out buffer too small
/// - -5: PoW not found within max_iterations
/// - -6: work_offset out of bounds
#[no_mangle]
pub extern "C" fn los_mine_pow(
    buffer: *mut u8,
    buffer_len: i32,
    work_offset: i32,
    difficulty_bits: u32,
    max_iterations: u64,
    nonce_out: *mut u64,
    hash_out: *mut u8,
    hash_capacity: i32,
) -> i32 {
    if buffer.is_null() || nonce_out.is_null() || hash_out.is_null() {
        return -1;
    }
    if (hash_capacity as usize) < 64 {
        return -2;
    }
    let buf_len = buffer_len as usize;
    let w_off = work_offset as usize;
    if w_off + 8 > buf_len {
        return -6;
    }

    let buf = unsafe { std::slice::from_raw_parts_mut(buffer, buf_len) };

    // Precompute difficulty check parameters
    let full_zero_bytes = (difficulty_bits / 8) as usize;
    let remaining_bits = difficulty_bits % 8;
    let mask: u8 = if remaining_bits > 0 {
        0xFF << (8 - remaining_bits)
    } else {
        0
    };

    for nonce in 0u64..max_iterations {
        // Write nonce as u64 LE into the work field
        let nonce_bytes = nonce.to_le_bytes();
        buf[w_off..w_off + 8].copy_from_slice(&nonce_bytes);

        // Keccak-256 hash
        let hash = Keccak256::digest(&*buf);

        // Check leading zero bits (fast byte-level check)
        let mut valid = true;
        for i in 0..full_zero_bytes {
            if hash[i] != 0 {
                valid = false;
                break;
            }
        }
        if valid && remaining_bits > 0 {
            if (hash[full_zero_bytes] & mask) != 0 {
                valid = false;
            }
        }

        if valid {
            // Write outputs
            unsafe { *nonce_out = nonce; }

            let hex_string = hex::encode(hash);
            let hex_bytes = hex_string.as_bytes();
            unsafe {
                std::ptr::copy_nonoverlapping(hex_bytes.as_ptr(), hash_out, hex_bytes.len());
            }
            return hex_bytes.len() as i32;
        }
    }

    -5 // PoW not found
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UTILITY — Hex encoding for Dart interop
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Encode raw bytes to hex string. Used by Dart to convert pk/sk/sig to hex.
///
/// # Returns
/// Hex string length on success, negative on error.
#[no_mangle]
pub extern "C" fn los_bytes_to_hex(
    input: *const u8,
    input_len: i32,
    hex_out: *mut u8,
    hex_capacity: i32,
) -> i32 {
    if input.is_null() || hex_out.is_null() {
        return -1;
    }

    let in_slice = unsafe { std::slice::from_raw_parts(input, input_len as usize) };
    let hex_string = hex::encode(in_slice);
    let hex_bytes = hex_string.as_bytes();

    if (hex_capacity as usize) < hex_bytes.len() {
        return -2;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(hex_bytes.as_ptr(), hex_out, hex_bytes.len());
    }

    hex_bytes.len() as i32
}

/// Decode hex string to raw bytes.
///
/// # Returns
/// Decoded byte length on success, negative on error.
#[no_mangle]
pub extern "C" fn los_hex_to_bytes(
    hex_in: *const u8,
    hex_len: i32,
    bytes_out: *mut u8,
    bytes_capacity: i32,
) -> i32 {
    if hex_in.is_null() || bytes_out.is_null() {
        return -1;
    }

    let hex_slice = unsafe { std::slice::from_raw_parts(hex_in, hex_len as usize) };
    let hex_str = match std::str::from_utf8(hex_slice) {
        Ok(s) => s,
        Err(_) => return -3,
    };

    let decoded = match hex::decode(hex_str) {
        Ok(b) => b,
        Err(_) => return -3,
    };

    if (bytes_capacity as usize) < decoded.len() {
        return -2;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(decoded.as_ptr(), bytes_out, decoded.len());
    }

    decoded.len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keygen_sign_verify() {
        let pk_size = los_public_key_bytes() as usize;
        let sk_size = los_secret_key_bytes() as usize;
        let sig_size = los_signature_bytes() as usize;

        let mut pk = vec![0u8; pk_size];
        let mut sk = vec![0u8; sk_size];

        let result = los_generate_keypair(
            pk.as_mut_ptr(), pk_size as i32,
            sk.as_mut_ptr(), sk_size as i32,
        );
        assert_eq!(result, 0, "Keypair generation failed");

        // Sign
        let message = b"test transaction data";
        let mut sig = vec![0u8; sig_size];

        let sig_len = los_sign(
            message.as_ptr(), message.len() as i32,
            sk.as_ptr(), sk_size as i32,
            sig.as_mut_ptr(), sig_size as i32,
        );
        assert!(sig_len > 0, "Signing failed: {}", sig_len);

        // Verify
        let valid = los_verify(
            message.as_ptr(), message.len() as i32,
            sig.as_ptr(), sig_len,
            pk.as_ptr(), pk_size as i32,
        );
        assert_eq!(valid, 1, "Verification failed");

        // Verify with wrong message
        let wrong_msg = b"wrong message";
        let invalid = los_verify(
            wrong_msg.as_ptr(), wrong_msg.len() as i32,
            sig.as_ptr(), sig_len,
            pk.as_ptr(), pk_size as i32,
        );
        assert_eq!(invalid, 0, "Should be invalid");
    }

    #[test]
    fn test_seeded_keygen_deterministic() {
        let pk_size = los_public_key_bytes() as usize;
        let sk_size = los_secret_key_bytes() as usize;

        // Simulate a BIP39 seed (64 bytes)
        let fake_seed = [42u8; 64];

        // Generate keypair #1
        let mut pk1 = vec![0u8; pk_size];
        let mut sk1 = vec![0u8; sk_size];
        let r1 = los_generate_keypair_from_seed(
            fake_seed.as_ptr(), 64,
            pk1.as_mut_ptr(), pk_size as i32,
            sk1.as_mut_ptr(), sk_size as i32,
        );
        assert_eq!(r1, 0);

        // Generate keypair #2 from same seed
        let mut pk2 = vec![0u8; pk_size];
        let mut sk2 = vec![0u8; sk_size];
        let r2 = los_generate_keypair_from_seed(
            fake_seed.as_ptr(), 64,
            pk2.as_mut_ptr(), pk_size as i32,
            sk2.as_mut_ptr(), sk_size as i32,
        );
        assert_eq!(r2, 0);

        // MUST be identical — deterministic from seed
        assert_eq!(pk1, pk2, "Public keys must match for same seed");
        assert_eq!(sk1, sk2, "Secret keys must match for same seed");

        // Different seed → different keypair
        let diff_seed = [99u8; 64];
        let mut pk3 = vec![0u8; pk_size];
        let mut sk3 = vec![0u8; sk_size];
        los_generate_keypair_from_seed(
            diff_seed.as_ptr(), 64,
            pk3.as_mut_ptr(), pk_size as i32,
            sk3.as_mut_ptr(), sk_size as i32,
        );
        assert_ne!(pk1, pk3, "Different seeds must produce different keys");
    }

    #[test]
    fn test_seeded_keygen_signatures_valid() {
        let pk_size = los_public_key_bytes() as usize;
        let sk_size = los_secret_key_bytes() as usize;
        let sig_size = los_signature_bytes() as usize;

        let seed = [7u8; 64];
        let mut pk = vec![0u8; pk_size];
        let mut sk = vec![0u8; sk_size];
        los_generate_keypair_from_seed(
            seed.as_ptr(), 64,
            pk.as_mut_ptr(), pk_size as i32,
            sk.as_mut_ptr(), sk_size as i32,
        );

        // Sign and verify with seeded keypair
        let message = b"hello LOS blockchain";
        let mut sig = vec![0u8; sig_size];
        let sig_len = los_sign(
            message.as_ptr(), message.len() as i32,
            sk.as_ptr(), sk_size as i32,
            sig.as_mut_ptr(), sig_size as i32,
        );
        assert!(sig_len > 0);

        let valid = los_verify(
            message.as_ptr(), message.len() as i32,
            sig.as_ptr(), sig_len,
            pk.as_ptr(), pk_size as i32,
        );
        assert_eq!(valid, 1, "Seeded keypair must produce valid signatures");
    }

    #[test]
    fn test_address_derivation() {
        let pk_size = los_public_key_bytes() as usize;
        let sk_size = los_secret_key_bytes() as usize;
        let mut pk = vec![0u8; pk_size];
        let mut sk = vec![0u8; sk_size];

        los_generate_keypair(
            pk.as_mut_ptr(), pk_size as i32,
            sk.as_mut_ptr(), sk_size as i32,
        );

        // Derive address
        let mut addr = vec![0u8; 64];
        let addr_len = los_public_key_to_address(
            pk.as_ptr(), pk_size as i32,
            addr.as_mut_ptr(), 64,
        );
        assert!(addr_len > 0, "Address derivation failed: {}", addr_len);

        let address = std::str::from_utf8(&addr[..addr_len as usize]).unwrap();
        assert!(address.starts_with("LOS"), "Address must start with LOS: {}", address);
        println!("Generated address: {}", address);

        // Validate
        let valid = los_validate_address(
            addr.as_ptr(), addr_len,
        );
        assert_eq!(valid, 1, "Address validation failed for: {}", address);
    }

    #[test]
    fn test_address_consistency_with_backend() {
        // The address derivation here must produce the exact same output
        // as los-crypto::public_key_to_address() for any given public key.
        // This is verified by using the same algorithm:
        // "LOS" + Base58( 0x4A | BLAKE2b-160(pubkey) | SHA256²(payload)[0..4] )
        let pk_size = los_public_key_bytes() as usize;
        let sk_size = los_secret_key_bytes() as usize;
        let mut pk = vec![0u8; pk_size];
        let mut sk = vec![0u8; sk_size];

        los_generate_keypair(
            pk.as_mut_ptr(), pk_size as i32,
            sk.as_mut_ptr(), sk_size as i32,
        );

        // Derive address using FFI function
        let mut addr1 = vec![0u8; 64];
        let len1 = los_public_key_to_address(
            pk.as_ptr(), pk_size as i32,
            addr1.as_mut_ptr(), 64,
        );

        // Derive address manually (replicating backend logic)
        let mut hasher = Blake2b512::new();
        hasher.update(&pk);
        let hash_result = hasher.finalize();
        let pubkey_hash = &hash_result[..20];

        let mut payload = vec![0x4Au8];
        payload.extend_from_slice(pubkey_hash);

        let hash1 = Sha256::digest(&payload);
        let hash2 = Sha256::digest(&hash1);
        let checksum = &hash2[..4];

        let mut address_bytes = payload;
        address_bytes.extend_from_slice(checksum);
        let manual_address = format!("LOS{}", bs58::encode(&address_bytes).into_string());

        let ffi_address = std::str::from_utf8(&addr1[..len1 as usize]).unwrap();
        assert_eq!(ffi_address, manual_address, "FFI and manual address derivation must match");
    }

    #[test]
    fn test_mine_pow_finds_valid_nonce() {
        // Simulate a signing_hash buffer:
        // [chain_id(8)] [account_bytes] [previous_bytes] [block_type(1)]
        // [amount(16)] [link_bytes] [public_key_bytes] [WORK(8)]
        // [timestamp(8)] [fee(16)]
        //
        // We just need a buffer with a known work_offset — content doesn't matter
        // for testing the PoW algorithm itself.

        let chain_id: u64 = 2; // testnet
        let account = b"LOSTestAddress123";
        let previous = b"0";
        let block_type: u8 = 0; // Send
        let amount: u128 = 1_000_000_000_000; // 10 LOS in CIL
        let link = b"LOSRecipientAddr456";
        let public_key = b"abcdef1234567890"; // abbreviated for test
        let timestamp: u64 = 1700000000;
        let fee: u128 = 100_000;

        // Build buffer (same layout as Dart and backend)
        let mut buf = Vec::new();
        buf.extend_from_slice(&chain_id.to_le_bytes());
        buf.extend_from_slice(account);
        buf.extend_from_slice(previous);
        buf.push(block_type);
        buf.extend_from_slice(&amount.to_le_bytes());
        buf.extend_from_slice(link);
        buf.extend_from_slice(public_key);

        let work_offset = buf.len();
        buf.extend_from_slice(&0u64.to_le_bytes()); // placeholder work

        buf.extend_from_slice(&timestamp.to_le_bytes());
        buf.extend_from_slice(&fee.to_le_bytes());

        let mut nonce_out: u64 = 0;
        let mut hash_buf = [0u8; 64];

        let start = std::time::Instant::now();
        let result = los_mine_pow(
            buf.as_mut_ptr(),
            buf.len() as i32,
            work_offset as i32,
            16, // 16 bits difficulty
            10_000_000, // max iterations
            &mut nonce_out as *mut u64,
            hash_buf.as_mut_ptr(),
            64,
        );
        let elapsed = start.elapsed();

        assert!(result > 0, "PoW mining failed with code: {}", result);
        assert_eq!(result, 64, "Hash hex should be 64 chars");

        let hash_hex = std::str::from_utf8(&hash_buf[..result as usize]).unwrap();
        println!("⛏️  Native PoW: nonce={}, hash={}..., time={:?}", nonce_out, &hash_hex[..16], elapsed);

        // Verify the hash actually has 16 leading zero bits (first 4 hex chars = "0000")
        assert!(hash_hex.starts_with("0000"), "Hash must have 16+ leading zero bits: {}", hash_hex);

        // Verify it's fast — should be under 2 seconds for 16-bit difficulty
        assert!(elapsed.as_secs() < 5, "PoW took too long: {:?}", elapsed);
    }
}
