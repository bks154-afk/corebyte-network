pub const TARGET_GAS_PER_BLOCK: u64 = 15_000_000;
pub const MAX_GAS_PER_BLOCK: u64 = 30_000_000;
pub const INITIAL_BASE_FEE_GWEI: f64 = 1.0;
pub const MIN_BASE_FEE_GWEI: f64 = 0.01;

pub fn calculate_next_base_fee(parent_gas_used: u64, parent_base_fee: f64) -> f64 {
    if parent_gas_used == TARGET_GAS_PER_BLOCK {
        return parent_base_fee.max(MIN_BASE_FEE_GWEI);
    }

    if parent_gas_used > TARGET_GAS_PER_BLOCK {
        let gas_delta = parent_gas_used - TARGET_GAS_PER_BLOCK;
        let fee_delta = parent_base_fee * (gas_delta as f64 / TARGET_GAS_PER_BLOCK as f64) / 8.0;
        (parent_base_fee + fee_delta).max(MIN_BASE_FEE_GWEI)
    } else {
        let gas_delta = TARGET_GAS_PER_BLOCK - parent_gas_used;
        let fee_delta = parent_base_fee * (gas_delta as f64 / TARGET_GAS_PER_BLOCK as f64) / 8.0;
        (parent_base_fee - fee_delta).max(MIN_BASE_FEE_GWEI)
    }
}