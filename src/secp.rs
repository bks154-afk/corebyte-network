use k256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use sha3::{Keccak256, Digest};

pub fn derive_address_from_pubkey(pubkey_hex: &str) -> Result<String, String> {
    let clean_hex = pubkey_hex.trim_start_matches("0x");
    let pubkey_bytes = hex::decode(clean_hex).map_err(|e| format!("Invalid hex: {}", e))?;
    
    let verifying_key = VerifyingKey::from_sec1_bytes(&pubkey_bytes)
        .map_err(|e| format!("Invalid Secp256k1 public key: {}", e))?;
    
    let uncompressed = verifying_key.to_encoded_point(false);
    let public_bytes = &uncompressed.as_bytes()[1..];

    let mut hasher = Keccak256::new();
    hasher.update(public_bytes);
    let hash = hasher.finalize();
    let address = &hash[12..];
    Ok(format!("0x{}", hex::encode(address)))
}

pub fn verify_signature(pubkey_hex: &str, message: &str, sig_hex: &str) -> bool {
    let clean_pub = pubkey_hex.trim_start_matches("0x");
    let clean_sig = sig_hex.trim_start_matches("0x");

    let pub_bytes = match hex::decode(clean_pub) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let sig_bytes = match hex::decode(clean_sig) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let verifying_key = match VerifyingKey::from_sec1_bytes(&pub_bytes) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let signature = match Signature::from_der(&sig_bytes).or_else(|_| Signature::from_slice(&sig_bytes)) {
        Ok(s) => s,
        Err(_) => return false,
    };

    verifying_key.verify(message.as_bytes(), &signature).is_ok()
}