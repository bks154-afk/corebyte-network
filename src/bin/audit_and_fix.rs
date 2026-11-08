use sled::Db;
use std::convert::TryInto;

const CBBITS_PER_CB: u128 = 100_000_000;

fn main() {
    let db_path = if std::path::Path::new("corebyte_db").exists() {
        "corebyte_db"
    } else if std::path::Path::new("block_planetary_mainnet.db").exists() {
        "block_planetary_mainnet.db"
    } else {
        "block_planetary_mainnet.db"
    };

    println!("===> Opening Sled DB: {}", db_path);
    let db = sled::open(db_path).expect("Failed to open DB");
    let blocks_tree = db.open_tree("blocks").expect("Failed to open blocks");
    let balances_tree = db.open_tree("account_balances").expect("Failed to open account_balances");

    let my_wallet = "0x34E01b6bC08cE82f88a516707B30289A40Af724c";
    let receiver = "0x9FBC8aFF75fA1082E1610E1cf3fd2ad378f01f33";

    let mut total_sent: f64 = 0.0;
    let mut total_fee: f64 = 0.0;
    let mut tx_count: u64 = 0;

    println!("===> Scanning all blocks for unauthorized transactions...");
    for item in blocks_tree.iter().flatten() {
        let val_bytes = item.1;
        if let Ok(block_json) = serde_json::from_slice::<serde_json::Value>(&val_bytes) {
            if let Some(txs) = block_json.get("transactions").and_then(|t| t.as_array()) {
                for tx in txs {
                    let sender = tx.get("sender").and_then(|s| s.as_str()).unwrap_or("");
                    let sig = tx.get("signature").and_then(|s| s.as_str()).unwrap_or("");
                    if sender.to_lowercase() == my_wallet.to_lowercase() && sig == "DEV_TEST_PASS" {
                        let amt = tx.get("amount").and_then(|a| a.as_f64()).unwrap_or(0.0);
                        let fee = tx.get("priority_fee").and_then(|f| f.as_f64()).unwrap_or(0.0);
                        total_sent += amt;
                        total_fee += fee;
                        tx_count += 1;
                        let height = block_json.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
                        println!("     [FOUND UNAUTHORIZED TX] Block #{}: Amount = {} CB, Fee = {} CB", height, amt, fee);
                    }
                }
            }
        }
    }

    println!("\n========================================================");
    println!("AUDIT SUMMARY FOR YOUR WALLET:");
    println!("Total Unauthorized Transactions : {}", tx_count);
    println!("Total Principal Amount Sent      : {} CB", total_sent);
    println!("Total Priority Fee Deducted      : {} CB", total_fee);
    let total_recredit = total_sent + total_fee;
    println!("Total CB to be Re-credited       : {} CB", total_recredit);
    println!("========================================================\n");

    if total_recredit > 0.0 {
        // Read current balance of my_wallet
        let my_curr_bits: u128 = balances_tree.get(my_wallet.as_bytes()).ok().flatten()
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);

        let add_bits = (total_recredit * CBBITS_PER_CB as f64) as u128;
        let new_my_bits = my_curr_bits + add_bits;
        balances_tree.insert(my_wallet.as_bytes(), new_my_bits.to_string().as_bytes()).unwrap();

        // Read and deduct from receiver
        let rec_curr_bits: u128 = balances_tree.get(receiver.as_bytes()).ok().flatten()
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);
        let deduct_bits = (total_sent * CBBITS_PER_CB as f64) as u128;
        let new_rec_bits = rec_curr_bits.saturating_sub(deduct_bits);
        balances_tree.insert(receiver.as_bytes(), new_rec_bits.to_string().as_bytes()).unwrap();

        db.flush().unwrap();
        println!("SUCCESS: Re-credited {} CB to your wallet!", total_recredit);
        println!("Old Balance: {} CB -> New Balance: {} CB", my_curr_bits as f64 / CBBITS_PER_CB as f64, new_my_bits as f64 / CBBITS_PER_CB as f64);
    } else {
        println!("No DEV_TEST_PASS transactions found. Balances are intact.");
    }
}
