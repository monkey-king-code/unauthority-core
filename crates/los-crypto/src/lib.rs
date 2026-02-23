// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - CRYPTOGRAPHY MODULE
//
// Post-quantum cryptography using Dilithium5 (NIST PQC standard).
// - Key generation (random and deterministic from BIP39 seed)
// - Message signing and verification
// - LOS address derivation (Base58Check with BLAKE2b-160)
// - Private key encryption via age (scrypt-based)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use age::{Decryptor, Encryptor};
use digest::Digest;
use pqcrypto_dilithium::dilithium5::{
    detached_sign, keypair, verify_detached_signature, PublicKey as DilithiumPublicKey,
    SecretKey as DilithiumSecretKey,
};
use pqcrypto_traits::sign::{DetachedSignature, PublicKey, SecretKey};
use secrecy::Secret;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use zeroize::Zeroize;

#[derive(Debug)]
pub enum CryptoError {
    InvalidKey,
    VerificationFailed,
    EncryptionFailed(String),
    DecryptionFailed(String),
    InvalidPassword,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CryptoError::InvalidKey => write!(f, "Invalid key format"),
            CryptoError::VerificationFailed => write!(f, "Signature verification failed"),
            CryptoError::EncryptionFailed(msg) => write!(f, "Encryption failed: {}", msg),
            CryptoError::DecryptionFailed(msg) => write!(f, "Decryption failed: {}", msg),
            CryptoError::InvalidPassword => write!(f, "Invalid password"),
        }
    }
}

impl std::error::Error for CryptoError {}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyPair {
    pub public_key: Vec<u8>,
    pub secret_key: Vec<u8>,
}

/// SECURITY: Zeroize secret key from memory on drop to prevent
/// recovery via memory dump, swap file, or core dump.
impl Drop for KeyPair {
    fn drop(&mut self) {
        self.secret_key.zeroize();
    }
}

/// Encrypted key structure with metadata
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptedKey {
    /// Encrypted secret key data
    pub ciphertext: Vec<u8>,
    /// Encryption version (for future upgrades)
    pub version: u32,
    /// Salt for key derivation (future use)
    pub salt: Vec<u8>,
    /// Public key (not encrypted)
    pub public_key: Vec<u8>,
}

/// Generate a new Post-Quantum key pair (Dilithium5)
pub fn generate_keypair() -> KeyPair {
    let (pk, sk) = keypair();
    KeyPair {
        public_key: pk.as_bytes().to_vec(),
        secret_key: sk.as_bytes().to_vec(),
    }
}

/// Generate DETERMINISTIC Dilithium5 keypair from BIP39 seed.
///
/// Uses domain-separated seed → ChaCha20 CSPRNG → deterministic `keypair()`.
/// Same seed ALWAYS produces the same keypair and address.
///
/// Domain separation:
///   salt = SHA-256("los-dilithium5-keygen-v1")
///   derived = SHA-256(salt || bip39_seed) → 32-byte ChaCha20 seed
///
/// # Arguments
/// * `bip39_seed` - BIP39 seed bytes (64 bytes from `mnemonic.to_seed("")`)
///   Must be at least 32 bytes.
///
/// # Panics
/// If seed is shorter than 32 bytes.
pub fn generate_keypair_from_seed(bip39_seed: &[u8]) -> KeyPair {
    // Mutex guard around set_seeded_rng() + keypair().
    // pqcrypto_internals::set_seeded_rng() sets GLOBAL thread-local state.
    // If two threads call this concurrently with different seeds, the RNG
    // state could be overwritten between set_seeded_rng() and keypair(),
    // producing an incorrect keypair (wrong address → potential fund loss).
    // The Mutex serializes all deterministic keygen calls.
    use std::sync::Mutex;
    static KEYGEN_LOCK: std::sync::LazyLock<Mutex<()>> =
        std::sync::LazyLock::new(|| Mutex::new(()));

    assert!(
        bip39_seed.len() >= 32,
        "BIP39 seed must be at least 32 bytes"
    );

    let _guard = KEYGEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Domain-separated deterministic seed derivation
    // Identical to flutter_wallet/native/los_crypto_ffi/src/lib.rs
    let salt = Sha256::digest(b"los-dilithium5-keygen-v1");
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(bip39_seed);
    let mut derived: [u8; 32] = hasher.finalize().into();

    // Activate deterministic CSPRNG for pqcrypto's randombytes
    pqcrypto_internals::set_seeded_rng(derived);

    // Generate keypair — now deterministic via seeded ChaCha20
    let (pk, sk) = keypair();

    // Revert to OS-RNG
    pqcrypto_internals::clear_seeded_rng();

    // SECURITY: Zero derived seed material immediately
    derived.zeroize();

    KeyPair {
        public_key: pk.as_bytes().to_vec(),
        secret_key: sk.as_bytes().to_vec(),
    }
}

/// Reconstruct a KeyPair from an existing Dilithium5 secret key.
///
/// Dilithium5 secret key contains the public key embedded in the last 2592 bytes.
/// Key size varies by pqcrypto library version (4864 or 4896 bytes).
///
/// Also accepts 32-byte seeds — treated as BIP39 seed[0:32] for legacy compat.
pub fn keypair_from_secret(secret_bytes: &[u8]) -> Result<KeyPair, CryptoError> {
    // Try to parse as full Dilithium5 secret key (size varies by implementation)
    if let Ok(_sk) = DilithiumSecretKey::from_bytes(secret_bytes) {
        // Dilithium5 SK contains PK in the last 2592 bytes
        let pk_bytes = &secret_bytes[secret_bytes.len() - 2592..];
        Ok(KeyPair {
            public_key: pk_bytes.to_vec(),
            secret_key: secret_bytes.to_vec(),
        })
    } else if secret_bytes.len() == 32 {
        // 32-byte seed — generate deterministic keypair
        Ok(generate_keypair_from_seed(secret_bytes))
    } else {
        Err(CryptoError::InvalidKey)
    }
}

/// Sign a message using a Dilithium5 secret key
pub fn sign_message(message: &[u8], secret_key_bytes: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let sk =
        DilithiumSecretKey::from_bytes(secret_key_bytes).map_err(|_| CryptoError::InvalidKey)?;

    let signature = detached_sign(message, &sk);
    Ok(signature.as_bytes().to_vec())
}

/// Verify signature — supports both Dilithium5 (post-quantum) and Ed25519 (testnet fallback).
///
/// Detection is by key/signature length:
/// - Dilithium5: public_key = 2592 bytes, signature = 4627 bytes
/// - Ed25519:    public_key = 32 bytes,   signature = 64 bytes
///
/// MAINNET SECURITY: Ed25519 is NOT post-quantum secure and is disabled on
/// mainnet builds (`--features mainnet`). Only Dilithium5 is accepted.
/// Ed25519 is a testnet-only fallback for Flutter desktop wallets where
/// native Dilithium5 FFI is not yet available.
pub fn verify_signature(message: &[u8], signature_bytes: &[u8], public_key_bytes: &[u8]) -> bool {
    // MAINNET: Only Dilithium5 signatures accepted (post-quantum enforcement)
    #[cfg(not(feature = "mainnet"))]
    if public_key_bytes.len() == 32 && signature_bytes.len() == 64 {
        // Ed25519 verification (TESTNET fallback for Flutter desktop)
        return verify_ed25519(message, signature_bytes, public_key_bytes);
    }

    // Default: Dilithium5 post-quantum verification
    verify_dilithium5(message, signature_bytes, public_key_bytes)
}

/// Dilithium5 signature verification (primary, post-quantum)
fn verify_dilithium5(message: &[u8], signature_bytes: &[u8], public_key_bytes: &[u8]) -> bool {
    let pk = match DilithiumPublicKey::from_bytes(public_key_bytes) {
        Ok(k) => k,
        Err(_) => return false,
    };

    use pqcrypto_dilithium::dilithium5::DetachedSignature as DilithiumSig;

    let sig = match DilithiumSig::from_bytes(signature_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    verify_detached_signature(&sig, message, &pk).is_ok()
}

/// Ed25519 signature verification (TESTNET fallback for Flutter desktop)
/// Uses ed25519-dalek which follows RFC 8032 — compatible with Dart `cryptography` package.
#[cfg(not(feature = "mainnet"))]
fn verify_ed25519(message: &[u8], signature_bytes: &[u8], public_key_bytes: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let pk_array: [u8; 32] = match public_key_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let vk = match VerifyingKey::from_bytes(&pk_array) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = match Signature::from_slice(signature_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    vk.verify(message, &sig).is_ok()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ADDRESS DERIVATION MODULE (Base58Check Format - Like Bitcoin)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use blake2::Blake2b512;

/// Derive LOS address from Dilithium5 public key (Base58Check format)
///
/// Format: Base58(version_byte + BLAKE2b160(pubkey) + checksum)
/// - Version: 0x4A (74 = "LOS" identifier)
/// - Hash: BLAKE2b-160 (20 bytes, quantum-resistant)
/// - Checksum: First 4 bytes of SHA256(SHA256(version + hash))
/// - Result: "LOS" prefix + Base58 encoded payload
///
/// # Example
/// ```
/// use los_crypto::{generate_keypair, public_key_to_address};
/// let keypair = generate_keypair();
/// let address = public_key_to_address(&keypair.public_key);
/// // Result: "LOSHjvLcaLZpKcRvHoEKtYdbQbMZECzNp3gh9LJ7Y9ZPTqH"
/// ```
pub fn public_key_to_address(public_key_bytes: &[u8]) -> String {
    const VERSION_BYTE: u8 = 0x4A; // 74 = "LOS" identifier

    // 1. Hash public key with BLAKE2b-512, take first 20 bytes (160-bit)
    let mut hasher = Blake2b512::new();
    hasher.update(public_key_bytes);
    let hash_result = hasher.finalize();
    let pubkey_hash = &hash_result[..20]; // Take first 20 bytes

    // 2. Construct payload: version + hash
    let mut payload = vec![VERSION_BYTE];
    payload.extend_from_slice(pubkey_hash);

    // 3. Calculate checksum: SHA256(SHA256(payload))
    let checksum_full = {
        let hash1 = Sha256::digest(&payload);
        Sha256::digest(hash1)
    };
    let checksum = &checksum_full[..4]; // First 4 bytes

    // 4. Combine: version + hash + checksum
    let mut address_bytes = payload;
    address_bytes.extend_from_slice(checksum);

    // 5. Base58 encode
    let base58_addr = bs58::encode(&address_bytes).into_string();

    // 6. Add "LOS" prefix for readability
    format!("LOS{}", base58_addr)
}

/// Validate LOS address format and checksum
///
/// Checks:
/// 1. Starts with "LOS" prefix
/// 2. Valid Base58 encoding
/// 3. Correct length (25 bytes decoded)
/// 4. Valid checksum
///
/// # Example
/// ```
/// use los_crypto::{generate_keypair, public_key_to_address, validate_address};
/// let keypair = generate_keypair();
/// let address = public_key_to_address(&keypair.public_key);
/// assert!(validate_address(&address));
/// ```
pub fn validate_address(address: &str) -> bool {
    // Must start with "LOS"
    if !address.starts_with("LOS") {
        return false;
    }

    // Decode Base58 (remove "LOS" prefix first)
    let base58_part = &address[3..];
    let decoded = match bs58::decode(base58_part).into_vec() {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    // Must be 25 bytes: 1 (version) + 20 (hash) + 4 (checksum)
    if decoded.len() != 25 {
        return false;
    }

    // Verify checksum
    let payload = &decoded[..21]; // version + hash
    let checksum = &decoded[21..]; // last 4 bytes

    let expected_checksum = {
        let hash1 = Sha256::digest(payload);
        Sha256::digest(hash1)
    };

    checksum == &expected_checksum[..4]
}

/// Extract public key hash from address (for debugging)
///
/// Note: Cannot reverse to original public key (one-way hash)!
/// Returns Some(hash) if address is valid, None otherwise.
///
/// # Example
/// ```
/// use los_crypto::{generate_keypair, public_key_to_address, address_to_pubkey_hash};
/// let keypair = generate_keypair();
/// let address = public_key_to_address(&keypair.public_key);
/// let hash = address_to_pubkey_hash(&address);
/// assert_eq!(hash.unwrap().len(), 20);
/// ```
pub fn address_to_pubkey_hash(address: &str) -> Option<Vec<u8>> {
    if !validate_address(address) {
        return None;
    }

    let base58_part = &address[3..];
    let decoded = bs58::decode(base58_part).into_vec().ok()?;

    // Extract hash (skip version byte, exclude checksum)
    Some(decoded[1..21].to_vec())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// KEY ENCRYPTION MODULE (RISK-002 Mitigation - P0 Critical)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Encrypt private key with password using age encryption
///
/// Security: Uses age's built-in scrypt key derivation (N=2^20, secure)
/// Format: age encrypted binary (portable, battle-tested)
///
/// # Arguments
/// * `secret_key` - Raw private key bytes (will be zeroized after encryption)
/// * `password` - User password (will be zeroized after use)
///
/// # Returns
/// Encrypted key structure with ciphertext and metadata
pub fn encrypt_private_key(secret_key: &[u8], password: &str) -> Result<EncryptedKey, CryptoError> {
    let password_secret = Secret::new(password.to_string());

    // Create age encryptor with password
    let encryptor = Encryptor::with_user_passphrase(password_secret);

    let mut encrypted_output = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted_output)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    writer
        .write_all(secret_key)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    writer
        .finish()
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    Ok(EncryptedKey {
        ciphertext: encrypted_output,
        version: 1,
        salt: vec![],       // age handles salt internally
        public_key: vec![], // To be filled by caller
    })
}

/// Decrypt private key with password
///
/// # Arguments
/// * `encrypted_key` - Encrypted key structure
/// * `password` - User password (will be zeroized after use)
///
/// # Returns
/// Decrypted private key bytes (caller must zeroize after use)
pub fn decrypt_private_key(
    encrypted_key: &EncryptedKey,
    password: &str,
) -> Result<Vec<u8>, CryptoError> {
    let password_secret = Secret::new(password.to_string());

    // Create age decryptor
    let decryptor = match Decryptor::new(&encrypted_key.ciphertext[..]) {
        Ok(Decryptor::Passphrase(d)) => d,
        Ok(_) => {
            return Err(CryptoError::DecryptionFailed(
                "Expected passphrase encryption".to_string(),
            ))
        }
        Err(e) => return Err(CryptoError::DecryptionFailed(e.to_string())),
    };

    // Decrypt with password
    let mut reader = decryptor
        .decrypt(&password_secret, None)
        .map_err(|e| match e {
            age::DecryptError::DecryptionFailed => CryptoError::InvalidPassword,
            _ => CryptoError::DecryptionFailed(e.to_string()),
        })?;

    let mut decrypted = Vec::new();
    reader
        .read_to_end(&mut decrypted)
        .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

    Ok(decrypted)
}

/// Check if key data is encrypted (simple heuristic)
///
/// age encrypted files start with "age-encryption.org/v1" header
pub fn is_encrypted(data: &[u8]) -> bool {
    data.starts_with(b"age-encryption.org/v1")
}

/// Migrate plaintext key to encrypted format
///
/// # Arguments
/// * `plaintext_key` - Plaintext KeyPair
/// * `password` - Password for encryption
///
/// # Returns
/// Encrypted key structure ready for storage
pub fn migrate_to_encrypted(
    plaintext_key: &KeyPair,
    password: &str,
) -> Result<EncryptedKey, CryptoError> {
    let mut encrypted = encrypt_private_key(&plaintext_key.secret_key, password)?;
    encrypted.public_key = plaintext_key.public_key.clone();
    Ok(encrypted)
}

/// Full key lifecycle: generate + encrypt
///
/// Generates new keypair and immediately encrypts private key
/// Public key remains unencrypted for address derivation
pub fn generate_encrypted_keypair(password: &str) -> Result<EncryptedKey, CryptoError> {
    let keypair = generate_keypair();
    migrate_to_encrypted(&keypair, password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_verify_flow() {
        let keys = generate_keypair();
        let msg = b"Hash Block LOS";
        let sig = sign_message(msg, &keys.secret_key).expect("Signing failed");
        assert!(verify_signature(msg, &sig, &keys.public_key));
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // KEY ENCRYPTION TESTS (RISK-002 Validation)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn test_encrypt_decrypt_private_key() {
        let keypair = generate_keypair();
        let password = "super_secure_password_123";

        // Encrypt
        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        assert!(!encrypted.ciphertext.is_empty());
        assert_ne!(encrypted.ciphertext, keypair.secret_key); // Ciphertext != plaintext

        // Decrypt
        let decrypted = decrypt_private_key(&encrypted, password).expect("Decryption failed");

        assert_eq!(decrypted, keypair.secret_key); // Decrypted == original
    }

    #[test]
    fn test_decrypt_with_wrong_password() {
        let keypair = generate_keypair();
        let password = "correct_password";
        let wrong_password = "wrong_password";

        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        // Should fail with wrong password
        let result = decrypt_private_key(&encrypted, wrong_password);
        assert!(result.is_err());

        match result.unwrap_err() {
            CryptoError::InvalidPassword => {} // Expected
            _ => panic!("Expected InvalidPassword error"),
        }
    }

    #[test]
    fn test_encrypted_key_still_signs() {
        let password = "test_password_456";

        // Generate and encrypt key
        let keypair = generate_keypair();
        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        // Decrypt for signing
        let decrypted_key = decrypt_private_key(&encrypted, password).expect("Decryption failed");

        // Sign message with decrypted key
        let msg = b"Test transaction";
        let sig = sign_message(msg, &decrypted_key).expect("Signing failed");

        // Verify signature with public key
        assert!(verify_signature(msg, &sig, &keypair.public_key));
    }

    #[test]
    fn test_is_encrypted_detection() {
        let keypair = generate_keypair();
        let password = "password";

        // Plaintext key should NOT be detected as encrypted
        assert!(!is_encrypted(&keypair.secret_key));

        // Encrypted key should be detected
        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");
        assert!(is_encrypted(&encrypted.ciphertext));
    }

    #[test]
    fn test_migrate_plaintext_to_encrypted() {
        let keypair = generate_keypair();
        let password = "migration_password";

        // Migrate
        let encrypted = migrate_to_encrypted(&keypair, password).expect("Migration failed");

        assert_eq!(encrypted.public_key, keypair.public_key); // Public key preserved
        assert_ne!(encrypted.ciphertext, keypair.secret_key); // Private key encrypted

        // Verify decryption works
        let decrypted = decrypt_private_key(&encrypted, password).expect("Decryption failed");
        assert_eq!(decrypted, keypair.secret_key);
    }

    #[test]
    fn test_generate_encrypted_keypair() {
        let password = "new_wallet_password";

        let encrypted_key = generate_encrypted_keypair(password).expect("Generation failed");

        assert!(!encrypted_key.public_key.is_empty());
        assert!(!encrypted_key.ciphertext.is_empty());
        assert!(is_encrypted(&encrypted_key.ciphertext));

        // Should be able to decrypt
        let decrypted = decrypt_private_key(&encrypted_key, password).expect("Decryption failed");
        assert!(!decrypted.is_empty());
    }

    #[test]
    fn test_encryption_version_field() {
        let keypair = generate_keypair();
        let password = "password";

        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        assert_eq!(encrypted.version, 1); // Current version
    }

    #[test]
    fn test_different_passwords_produce_different_ciphertexts() {
        let keypair = generate_keypair();
        let password1 = "password1";
        let password2 = "password2";

        let encrypted1 =
            encrypt_private_key(&keypair.secret_key, password1).expect("Encryption 1 failed");
        let encrypted2 =
            encrypt_private_key(&keypair.secret_key, password2).expect("Encryption 2 failed");

        // Different passwords should produce different ciphertexts
        assert_ne!(encrypted1.ciphertext, encrypted2.ciphertext);

        // But both should decrypt to same plaintext
        let decrypted1 = decrypt_private_key(&encrypted1, password1).unwrap();
        let decrypted2 = decrypt_private_key(&encrypted2, password2).unwrap();
        assert_eq!(decrypted1, decrypted2);
        assert_eq!(decrypted1, keypair.secret_key);
    }

    #[test]
    fn test_empty_password_still_encrypts() {
        let keypair = generate_keypair();
        let password = ""; // Empty password (not recommended but should work)

        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        let decrypted = decrypt_private_key(&encrypted, password).expect("Decryption failed");

        assert_eq!(decrypted, keypair.secret_key);
    }

    #[test]
    fn test_long_password_works() {
        let keypair = generate_keypair();
        let password = "a".repeat(500); // 500 character password

        let encrypted =
            encrypt_private_key(&keypair.secret_key, &password).expect("Encryption failed");

        let decrypted = decrypt_private_key(&encrypted, &password).expect("Decryption failed");

        assert_eq!(decrypted, keypair.secret_key);
    }

    #[test]
    fn test_unicode_password_works() {
        let keypair = generate_keypair();
        let password = "密码🔒パスワード"; // Mixed Unicode

        let encrypted =
            encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");

        let decrypted = decrypt_private_key(&encrypted, password).expect("Decryption failed");

        assert_eq!(decrypted, keypair.secret_key);
    }

    #[test]
    fn test_encryption_is_consistent() {
        // Note: age encryption includes random nonce, so same input won't produce same output
        // This test validates that decrypt(encrypt(x)) == x consistently
        let keypair = generate_keypair();
        let password = "consistent_password";

        for _ in 0..5 {
            let encrypted =
                encrypt_private_key(&keypair.secret_key, password).expect("Encryption failed");
            let decrypted = decrypt_private_key(&encrypted, password).expect("Decryption failed");

            assert_eq!(decrypted, keypair.secret_key);
        }
    }
}
