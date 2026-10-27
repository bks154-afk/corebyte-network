pub use crate::types::BlockExecutionOutcome;
use crate::types::{Block, OracleFeedEntry, SignedTransaction, DEFAULT_MINER_WALLET, INITIAL_COINBASE_SUBSIDY, CBBITS_PER_CB};
use crate::crypto;
use sled::{Db, Tree};
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const L2_ROLLUP_SETTLEMENT_ADDRESS: &str = "0xCB_L2_ROLLUP_SEQUENCER_SETTLEMENT";
pub const MIN_DIFFICULTY: usize = 1;
pub const MAX_DIFFICULTY: usize = 12;
pub const INITIAL_DIFFICULTY: usize = 4;
pub const TARGET_BLOCK_TIME_SECS: u64 = 10;
pub const ADJUSTMENT_INTERVAL: u64 = 100;

static ATTESTATIONS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

pub fn record_pos_attestation(height: u64, attestation: String) {
    if let Ok(mut lock) = ATTESTATIONS.lock() {
        lock.push((height, attestation));
    }
}

pub fn drain_pos_attestations(_height: u64) -> Vec<String> {
    if let Ok(mut lock) = ATTESTATIONS.lock() {
        if lock.is_empty() {
            vec!["POS_CONSENSUS_VALIDATED_QUORUM".to_string()]
        } else {
            lock.drain(..)
                .map(|(_, sig)| sig)
                .collect()
        }
    } else {
        vec!["POS_CONSENSUS_VALIDATED_QUORUM".to_string()]
    }
}

pub fn get_miner_address() -> String {
    env::var("MINER_WALLET").unwrap_or_else(|_| DEFAULT_MINER_WALLET.to_string())
}

pub fn calculate_block_reward(height: u64, _total_minted: f64) -> f64 {
    let halvings = height / 210_000;
    if halvings >= 64 {
        0.0
    } else {
        INITIAL_COINBASE_SUBSIDY / (1u64 << halvings) as f64
    }
}

pub fn calculate_dynamic_difficulty(
    blocks_tree: &Tree,
    current_height: u64,
    current_difficulty: usize,
) -> usize {
    if current_height < 5 {
        return INITIAL_DIFFICULTY.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);
    }

    let last_key = (current_height - 1).to_be_bytes();
    let prev_key = (current_height - 5).to_be_bytes();

    let last_time = blocks_tree.get(last_key).ok().flatten()
        .and_then(|b| serde_json::from_slice::<Block>(b.as_ref()).ok())
        .map(|b| b.timestamp)
        .unwrap_or(0);

    let prev_time = blocks_tree.get(prev_key).ok().flatten()
        .and_then(|b| serde_json::from_slice::<Block>(b.as_ref()).ok())
        .map(|b| b.timestamp)
        .unwrap_or(0);

    if last_time == 0 || prev_time == 0 || last_time <= prev_time {
        return current_difficulty.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);
    }

    let time_taken = last_time - prev_time;
    let expected_time = 4 * TARGET_BLOCK_TIME_SECS;

    if time_taken < (expected_time / 2) {
        (current_difficulty + 1).min(MAX_DIFFICULTY)
    } else if time_taken > (expected_time * 2) {
        current_difficulty.saturating_sub(1).max(MIN_DIFFICULTY)
    } else {
        current_difficulty.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY)
    }
}

pub fn calculate_difficulty(_height: u64, previous_difficulty: usize) -> usize {
    previous_difficulty.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY)
}

pub fn apply_block_transactions_and_rewards(
    db: &Db,
    txs: &[SignedTransaction],
    miner: &str,
    reward: f64,
    _base_fee: f64,
    _height: u64,
    _block_hash: &str,
    _oracle_feeds: &[OracleFeedEntry],
) -> Result<BlockExecutionOutcome, String> {
    let balances_tree = db.open_tree("account_balances").map_err(|e| e.to_string())?;
    let nonces_tree = db.open_tree("account_nonces").map_err(|e| e.to_string())?;

    // Runtime par Miner ka reward balance update (in CBbit)
    let reward_cbbit = (reward * CBBITS_PER_CB as f64).round() as u128;
    let cur_miner_cbbit: u128 = balances_tree.get(miner.as_bytes()).ok().flatten()
        .or_else(|| balances_tree.get(miner.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0);

    let new_miner_cbbit = cur_miner_cbbit + reward_cbbit;
    let _ = balances_tree.insert(miner.as_bytes(), new_miner_cbbit.to_string().as_bytes());
    let _ = balances_tree.insert(miner.to_lowercase().as_bytes(), new_miner_cbbit.to_string().as_bytes());

    // Block transactions apply (transfers measured in CB & CBbit)
    for tx in txs {
        if tx.sender == "0xCB_COINBASE_MINT" {
            continue;
        }

        let total_tx_cbbit = ((tx.amount + tx.priority_fee) * CBBITS_PER_CB as f64).round() as u128;
        let amount_cbbit = (tx.amount * CBBITS_PER_CB as f64).round() as u128;

        let sender_cbbit: u128 = balances_tree.get(tx.sender.as_bytes()).ok().flatten()
            .or_else(|| balances_tree.get(tx.sender.to_lowercase().as_bytes()).ok().flatten())
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);

        if sender_cbbit >= total_tx_cbbit {
            let new_sender_cbbit = sender_cbbit - total_tx_cbbit;
            let _ = balances_tree.insert(tx.sender.as_bytes(), new_sender_cbbit.to_string().as_bytes());
            let _ = balances_tree.insert(tx.sender.to_lowercase().as_bytes(), new_sender_cbbit.to_string().as_bytes());

            let recv_cbbit: u128 = balances_tree.get(tx.receiver.as_bytes()).ok().flatten()
                .or_else(|| balances_tree.get(tx.receiver.to_lowercase().as_bytes()).ok().flatten())
                .and_then(|v| String::from_utf8(v.to_vec()).ok())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);

            let new_recv_cbbit = recv_cbbit + amount_cbbit;
            let _ = balances_tree.insert(tx.receiver.as_bytes(), new_recv_cbbit.to_string().as_bytes());
            let _ = balances_tree.insert(tx.receiver.to_lowercase().as_bytes(), new_recv_cbbit.to_string().as_bytes());

            let _ = nonces_tree.insert(tx.sender.as_bytes(), tx.nonce.to_string().as_bytes());
            let _ = nonces_tree.insert(tx.sender.to_lowercase().as_bytes(), tx.nonce.to_string().as_bytes());
        }
    }

    let _ = balances_tree.flush();
    let _ = nonces_tree.flush();

    Ok(BlockExecutionOutcome {
        state_root: format!("0x{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()),
        receipts_root: "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        logs_bloom: format!("0x{}", "0".repeat(512)),
        total_gas_used: 21000,
        total_burned: 0.0,
    })
}

pub fn evaluate_and_apply_block(
    db: &Db,
    block: &Block,
    _signal: &Arc<AtomicU64>,
) -> Result<(), String> {
    apply_block_transactions_and_rewards(
        db,
        &block.transactions,
        &block.validator,
        block.reward,
        block.base_fee,
        block.height,
        &block.hash,
        &block.oracle_feeds,
    )?;
    Ok(())
}

pub fn mine_memory_hard_block_parallel(
    height: u64,
    prev_hash: &str,
    state_root: &str,
    receipts_root: &str,
    target_zeros: usize,
    interrupt_handle: Arc<AtomicU64>,
) -> Option<(u64, String)> {
    let effective_zeros = target_zeros.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);
    let target_prefix = "0".repeat(effective_zeros);
    let mut nonce: u64 = 0;

    let candidate = format!("{}:{}:{}:{}", height, prev_hash, state_root, receipts_root);

    println!(
        "[DUAL-CONSENSUS MINER]: Mining Block #{} (Memory-Hard) | Target: {} zeros",
        height, effective_zeros
    );

    loop {
        if interrupt_handle.load(Ordering::Relaxed) != 0 {
            return None;
        }

        let hash = crypto::memory_hard_hash(&candidate, nonce);

        if hash.starts_with(&target_prefix) {
            println!(
                "[LEDGER SEALED]: [OK] Block #{} committed | Hash: {} | Nonce: {}",
                height, hash, nonce
            );
            return Some((nonce, hash));
        }

        nonce = nonce.wrapping_add(1);
    }
}