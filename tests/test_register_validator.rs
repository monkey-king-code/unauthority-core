use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Generate fresh Dilithium5 keypair
    let kp = los_crypto::generate_keypair();
    let address = los_crypto::public_key_to_address(&kp.public_key);
    let pk_hex = hex::encode(&kp.public_key);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let message = format!("REGISTER_VALIDATOR:{}:{}", address, timestamp);
    let signature =
        los_crypto::sign_message(message.as_bytes(), &kp.secret_key).expect("sign failed");
    let sig_hex = hex::encode(&signature);

    // Output JSON payload
    println!(
        "{{\"address\":\"{}\",\"public_key\":\"{}\",\"signature\":\"{}\",\"timestamp\":{}}}",
        address, pk_hex, sig_hex, timestamp
    );
}
