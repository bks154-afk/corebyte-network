use sha2::{Sha512, Digest};

pub fn calculate_merkle_root(entries: &mut [(String, String)]) -> String {
    if entries.is_empty() {
        return "0x0000000000000000000000000000000000000000000000000000000000000000".to_string();
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut current_level: Vec<String> = entries.iter().map(|(k, v)| {
        let mut hasher = Sha512::new();
        hasher.update(format!("{}:{}", k, v).as_bytes());
        format!("{:x}", hasher.finalize())
    }).collect();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in current_level.chunks(2) {
            let mut hasher = Sha512::new();
            if chunk.len() == 2 {
                hasher.update(format!("{}{}", chunk[0], chunk[1]).as_bytes());
            } else {
                hasher.update(format!("{}{}", chunk[0], chunk[0]).as_bytes());
            }
            next_level.push(format!("{:x}", hasher.finalize()));
        }
        current_level = next_level;
    }
    format!("0x{}", &current_level[0][..64])
}

pub fn generate_merkle_proof(entries: &mut [(String, String)], target_key: &str) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut proof = Vec::new();
    let target_idx = entries.iter().position(|(k, _)| k == target_key);

    if let Some(idx) = target_idx {
        let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx.saturating_sub(1) };
        if sibling_idx < entries.len() {
            proof.push(format!("SiblingNode:{}:{}", entries[sibling_idx].0, entries[sibling_idx].1));
        }
    }
    proof
}