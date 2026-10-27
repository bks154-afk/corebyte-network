use sha2::{Sha256, Sha512, Digest};

pub fn hash_quantum_secure(input: &str) -> String {
    let mut hasher1 = Sha512::new();
    hasher1.update(input.as_bytes());
    let round1 = hasher1.finalize();

    let mut hasher2 = Sha512::new();
    hasher2.update(round1);
    format!("{:x}", hasher2.finalize())
}

pub fn memory_hard_hash(candidate: &str, nonce: u64) -> String {
    const SCRATCHPAD_ITEMS: usize = 1024;
    let seed = format!("{}:{}", candidate, nonce);
    let mut state = hash_quantum_secure(&seed);

    let mut scratchpad = Vec::with_capacity(SCRATCHPAD_ITEMS);
    for i in 0..SCRATCHPAD_ITEMS {
        let mut hasher = Sha256::new();
        hasher.update(state.as_bytes());
        hasher.update(&(i as u64).to_be_bytes());
        let cell = format!("{:x}", hasher.finalize());
        scratchpad.push(cell.clone());
        state = cell;
    }

    let mut final_hasher = Sha512::new();
    let mut index = (nonce as usize) % SCRATCHPAD_ITEMS;
    for _ in 0..64 {
        let item = &scratchpad[index];
        final_hasher.update(item.as_bytes());
        let byte_val = item.as_bytes()[0] as usize;
        index = (index + byte_val) % SCRATCHPAD_ITEMS;
    }

    format!("{:x}", final_hasher.finalize())
}