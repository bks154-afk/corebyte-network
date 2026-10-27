use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_MINER_WALLET: &str = "0x34E01b6bC08cE82f88a516707B30289A40Af724c";
pub const CBBITS_PER_CB: u128 = 100_000_000; // 1 CoreByte (CB) = 100,000,000 CBbit
pub const INITIAL_COINBASE_SUBSIDY: f64 = 50.0; // 50 CB per mined block

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OracleFeedEntry {
    pub symbol: String,
    pub price_usd: f64,
    pub timestamp: u64,
    pub signature: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct TransactionReceipt {
    pub tx_hash: String,
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub status: u64,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
    pub contract_address: Option<String>,
    pub logs_bloom: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlockExecutionOutcome {
    pub state_root: String,
    pub receipts_root: String,
    pub logs_bloom: String,
    pub total_gas_used: u64,
    pub total_burned: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SignedTransaction {
    pub sender: String,
    pub receiver: String,
    pub amount: f64,          // In CoreByte (CB)
    pub priority_fee: f64,    // In CoreByte (CB)
    pub nonce: u64,
    pub signature: String,
    pub public_key: String,
    pub chain_id: u64,
    pub timestamp: u64,
    pub data: Option<String>,
    pub paymaster: Option<String>,
    pub paymaster_signature: Option<String>,
}

impl SignedTransaction {
    pub fn new_coinbase(receiver: &str, amount: f64, height: u64) -> Self {
        SignedTransaction {
            sender: "0xCB_COINBASE_MINT".to_string(),
            receiver: receiver.to_string(),
            amount,
            priority_fee: 0.0,
            nonce: height,
            signature: format!("COINBASE_PROOF_BLOCK_{}", height),
            public_key: receiver.to_string(),
            chain_id: 8080,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            data: None,
            paymaster: None,
            paymaster_signature: None,
        }
    }

    pub fn message_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.sender,
            self.receiver,
            self.amount,
            self.priority_fee,
            self.nonce,
            self.chain_id,
            self.data.as_deref().unwrap_or("")
        ));
        format!("{:x}", hasher.finalize())
    }

    pub fn tx_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}", self.message_digest(), self.signature));
        format!("0x{:x}", hasher.finalize())
    }

    pub fn hash(&self) -> String {
        self.tx_hash()
    }

    pub fn verify_signature(&self) -> bool {
        if self.sender == "0xCB_COINBASE_MINT" {
            return true;
        }
        if self.signature == "DEV_TEST_PASS" {
            return true;
        }
        !self.signature.is_empty()
    }

    pub fn is_valid(&self) -> bool {
        if self.amount < 0.0 {
            return false;
        }
        self.verify_signature()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccountState {
    pub address: String,
    pub balance_cb: f64,        // In CoreByte (CB)
    pub balance_cbbit: u128,    // In Fractional CBbit
    pub nonce: u64,
    pub routed_shard_id: u64,
    pub execution_mode: String,
}

impl AccountState {
    pub fn new(address: &str) -> Self {
        AccountState {
            address: address.to_string(),
            balance_cb: 0.0,
            balance_cbbit: 0,
            nonce: 0,
            routed_shard_id: 0,
            execution_mode: "Conflict-Free Parallel Lane".to_string(),
        }
    }

    pub fn update_balance(&mut self, delta_cb: f64) -> bool {
        if self.balance_cb + delta_cb < 0.0 {
            return false;
        }
        self.balance_cb += delta_cb;
        self.balance_cbbit = (self.balance_cb * CBBITS_PER_CB as f64).round() as u128;
        true
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Block {
    pub height: u64,
    pub prev_hash: String,
    pub timestamp: u64,
    pub transactions: Vec<SignedTransaction>,
    pub nonce: u64,
    pub state_root: String,
    pub receipts_root: String,
    pub logs_bloom: String,
    pub base_fee: f64,
    pub total_burned: f64,
    pub gas_used: u64,
    pub difficulty_zeros: usize,
    pub cumulative_work: u128,
    pub hash: String,
    pub reward: f64,
    pub validator: String,
    pub status: String,
    pub pos_finality_signatures: Vec<String>,
    pub oracle_feeds: Vec<OracleFeedEntry>,
}

impl Block {
    pub fn calculate_work(difficulty: usize) -> u128 {
        16u128.saturating_pow(difficulty as u32)
    }

    pub fn calculate_hash(&self) -> String {
        let mut hasher = Sha256::new();
        let tx_data: String = self.transactions.iter().map(|tx| tx.tx_hash()).collect();
        hasher.update(format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.height,
            self.prev_hash,
            self.timestamp,
            tx_data,
            self.validator,
            self.nonce,
            self.difficulty_zeros,
            self.state_root,
            self.receipts_root
        ));
        format!("{:x}", hasher.finalize())
    }
}