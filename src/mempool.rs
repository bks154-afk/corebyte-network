use crate::types::{SignedTransaction, CBBITS_PER_CB};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct PersistentMempool {
    tree: sled::Tree,
}

impl PersistentMempool {
    pub fn new(db: &sled::Db) -> Self {
        Self {
            tree: db.open_tree("pending_mempool").unwrap(),
        }
    }

    pub fn insert_with_rbf(&self, tx: SignedTransaction) -> Result<String, String> {
        if tx.chain_id != 8080 {
            return Err(format!("Replay Protection: Invalid chain_id {}, expected 8080.", tx.chain_id));
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        for item in self.tree.iter().flatten() {
            if let Ok(queued) = serde_json::from_slice::<SignedTransaction>(item.1.as_ref()) {
                if now.saturating_sub(queued.timestamp) > 86400 {
                    let _ = self.tree.remove(item.0);
                }
            }
        }

        if self.tree.len() >= 50_000 {
            if let Some(lowest_entry) = self.tree.iter().flatten().last() {
                if let Ok(lowest_tx) = serde_json::from_slice::<SignedTransaction>(lowest_entry.1.as_ref()) {
                    if tx.priority_fee > lowest_tx.priority_fee {
                        let _ = self.tree.remove(lowest_entry.0);
                    } else {
                        return Err("Mempool saturated: Fee too low for priority inclusion.".to_string());
                    }
                }
            }
        }

        let mut existing_key = None;
        let mut can_replace = false;

        for item in self.tree.iter().flatten() {
            if let Ok(queued_tx) = serde_json::from_slice::<SignedTransaction>(item.1.as_ref()) {
                if queued_tx.sender.to_lowercase() == tx.sender.to_lowercase() && queued_tx.nonce == tx.nonce {
                    existing_key = Some(item.0);
                    if tx.priority_fee >= queued_tx.priority_fee * 1.10 {
                        can_replace = true;
                    }
                    break;
                }
            }
        }

        if let Some(key) = existing_key {
            if can_replace {
                let _ = self.tree.remove(key);
            } else {
                return Err("RBF rejection: replacement transaction must provide >= +10% priority fee.".to_string());
            }
        }

        let key = format!("{:016.4}_{}", (10000.0 - tx.priority_fee).max(0.0), tx.message_digest());
        let val = serde_json::to_vec(&tx).map_err(|e| e.to_string())?;
        self.tree.insert(key.as_bytes(), val).map_err(|e| e.to_string())?;
        let _ = self.tree.flush();
        Ok(tx.tx_hash())
    }

    pub fn peek_top_n(&self, n: usize) -> (Vec<SignedTransaction>, Vec<sled::IVec>) {
        let mut txs = Vec::new();
        let mut keys = Vec::new();

        for item in self.tree.iter().flatten().take(n) {
            if let Ok(tx) = serde_json::from_slice::<SignedTransaction>(item.1.as_ref()) {
                keys.push(item.0);
                txs.push(tx);
            }
        }
        (txs, keys)
    }

    pub fn purge_keys(&self, keys: &[sled::IVec]) {
        for k in keys {
            let _ = self.tree.remove(k);
        }
        let _ = self.tree.flush();
    }

    pub fn count(&self) -> usize {
        self.tree.len()
    }
}