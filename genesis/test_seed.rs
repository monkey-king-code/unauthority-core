// Test to verify Rust genesis seed derivation
use bip39::{Language, Mnemonic};
use ed25519_dalek::{SigningKey, VerifyingKey};

fn main() {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║   RUST GENESIS ADDRESS DERIVATION TEST                    ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    // Dev Wallet #1 from genesis
    let mnemonic_str = "riot draft insect furnace soldier faith recipe fabric auction public select diamond arrow topple naive wheel opinion kit thumb noble guitar addict monkey pipe";
    let expected_address = "LOSe8ef3d432398019ae91c6f374edd07f4a2c5bfcb";

    println!("Testing: Dev Wallet #1");
    println!("─────────────────────────────────────────────────────────────");
    println!("Mnemonic: {}...", &mnemonic_str[..50]);
    
    // Parse mnemonic
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .expect("Invalid mnemonic");
    
    // Generate seed with EMPTY passphrase
    let seed = mnemonic.to_seed("");
    
    println!("\nSeed (first 64 bytes hex):");
    for (i, chunk) in seed.chunks(32).take(2).enumerate() {
        print!("  [{}] ", i * 32);
        for byte in chunk {
            print!("{:02x}", byte);
        }
        println!();
    }
    
    // Derive Ed25519 keypair
    let secret_key = SigningKey::from_bytes(&seed[0..32].try_into().unwrap());
    let public_key: VerifyingKey = secret_key.verifying_key();
    
    println!("\nPrivate key (32 bytes hex):");
    print!("  ");
    for byte in secret_key.to_bytes() {
        print!("{:02x}", byte);
    }
    println!();
    
    println!("\nPublic key (32 bytes hex):");
    print!("  ");
    for byte in public_key.as_bytes() {
        print!("{:02x}", byte);
    }
    println!();
    
    // Derive address
    let address = format!("LOS{}", bs58::encode(public_key.as_bytes()).into_string());
    
    println!("\nExpected address: {}", expected_address);
    println!("Derived address:  {}", address);
    
    if address == expected_address {
        println!("\n✅ MATCH!");
    } else {
        println!("\n❌ MISMATCH!");
    }
    
    println!("─────────────────────────────────────────────────────────────\n");
}
