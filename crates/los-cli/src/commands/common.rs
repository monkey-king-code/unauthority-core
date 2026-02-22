use std::path::Path;

/// Shared wallet loader — loads wallet file from config_dir/wallets/{name}.json,
/// prompts for password, decrypts secret key, derives & verifies address.
/// Used by both `tx` and `validator` commands.
pub fn load_wallet_keypair(
    wallet_name: &str,
    config_dir: &Path,
) -> Result<(String, los_crypto::KeyPair), Box<dyn std::error::Error>> {
    let wallet_file = config_dir
        .join("wallets")
        .join(format!("{}.json", wallet_name));
    if !wallet_file.exists() {
        return Err(format!(
            "Wallet '{}' not found at {}",
            wallet_name,
            wallet_file.display()
        )
        .into());
    }

    let data = std::fs::read_to_string(&wallet_file)?;
    let wallet: serde_json::Value = serde_json::from_str(&data)?;

    let address = wallet["address"]
        .as_str()
        .ok_or("Wallet file missing 'address' field")?
        .to_string();

    // Password from env var (for automation/scripting) or interactive prompt
    let password = match std::env::var("LOS_WALLET_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => rpassword::prompt_password("Enter wallet password: ")?,
    };

    let encrypted_key: los_crypto::EncryptedKey =
        serde_json::from_value(wallet["encrypted_key"].clone())
            .map_err(|e| format!("Invalid encrypted_key in wallet file: {}", e))?;

    let secret_bytes = los_crypto::decrypt_private_key(&encrypted_key, &password)
        .map_err(|e| format!("Decryption failed (wrong password?): {:?}", e))?;

    // Build keypair using the stored public key (known correct) and decrypted secret
    // We prefer using the stored public_key rather than extracting from SK
    // because SK layout varies between pqcrypto library versions
    let keypair = if !encrypted_key.public_key.is_empty() {
        // Validate the secret key is parse-able by the crypto library
        los_crypto::sign_message(b"validate", &secret_bytes)
            .map_err(|_| "Decrypted secret key invalid (cannot sign)")?;
        los_crypto::KeyPair {
            public_key: encrypted_key.public_key.clone(),
            secret_key: secret_bytes,
        }
    } else {
        los_crypto::keypair_from_secret(&secret_bytes).map_err(|_| {
            format!(
                "Decrypted key has invalid format (len={})",
                secret_bytes.len()
            )
        })?
    };

    // Verify derived address matches stored address
    let derived_addr = los_crypto::public_key_to_address(&keypair.public_key);
    if derived_addr != address {
        return Err("Decrypted key does not match wallet address!".into());
    }

    Ok((address, keypair))
}
