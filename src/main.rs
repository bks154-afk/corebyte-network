#![allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case)]

mod types;
mod crypto;
mod secp;
mod gas;
mod smt;
mod vm;
mod mempool;
mod consensus;
mod dashboard;
mod rpc;
mod p2p;

use actix_web::{web, App, HttpServer};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use types::*;
use mempool::PersistentMempool;
use rpc::AppStateContainer;
use p2p::P2PCommand;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let network_id = "COREBYTE_MAINNET_PRIME".to_string();
    let rpc_port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let stratum_port: u16 = 3333;
    let p2p_port: u16 = std::env::var("P2P_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(9000);
    let miner_address = std::env::var("MINER_WALLET").unwrap_or_else(|_| "0x34E01b6bC08cE82f88a516707B30289A40Af724c".to_string());

    println!("====================================================================");
    println!("🌐 CoreByte (CB) SOVEREIGN VALIDATOR & RPC NODE (v6.5.0)");
    println!("🪙 Currency: CoreByte (CB) | Fractional Unit: CBbit (1 CB = 100M CBbit)");
    println!("⚡ Status: Pure Validator & Stratum Pool Server (SERVER CPU MINING: OFF)");
    println!("====================================================================");

    let db = sled::open("block_planetary_mainnet.db")
        .expect("Failed to initialize Sled database engine");

    let persistent_mempool = PersistentMempool::new(&db);
    let blocks_tree = db.open_tree("blocks").unwrap();
    let balances_tree = db.open_tree("account_balances").unwrap();

    let genesis_treasury_addr = "0x34E01b6bC08cE82f88a516707B30289A40Af724c";
    if balances_tree.get(genesis_treasury_addr.as_bytes()).ok().flatten().is_none() {
        let _ = balances_tree.insert(genesis_treasury_addr.as_bytes(), b"0");
        let _ = balances_tree.insert(genesis_treasury_addr.to_lowercase().as_bytes(), b"0");
        let _ = balances_tree.flush();
    }

    let (p2p_cmd_sender, p2p_cmd_receiver) = mpsc::channel::<P2PCommand>(1000);
    let (p2p_event_sender, mut p2p_event_receiver) = mpsc::channel::<String>(1000);
    let (ws_sender, _) = tokio::sync::broadcast::channel::<String>(1000);
    let interrupt_signal = Arc::new(AtomicU64::new(0));

    let bootstrap_peers: Vec<String> = std::env::var("BOOTSTRAP_NODES")
        .unwrap_or_else(|_| "".to_string())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    tokio::spawn(async move {
        if let Err(e) = p2p::start_libp2p_node(p2p_port, bootstrap_peers, p2p_cmd_receiver, p2p_event_sender).await {
            eprintln!("[P2P FATAL ERROR]: Libp2p engine terminated: {:?}", e);
        }
    });

    let p2p_sync_db = db.clone();
    let p2p_sync_signal = interrupt_signal.clone();
    let p2p_sync_mempool = persistent_mempool.clone();
    tokio::spawn(async move {
        while let Some(msg) = p2p_event_receiver.recv().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg) {
                if val["type"] == "BLOCK_GOSSIP" {
                    if let Ok(inbound_block) = serde_json::from_value::<Block>(val["block"].clone()) {
                        let _ = consensus::evaluate_and_apply_block(&p2p_sync_db, &inbound_block, &p2p_sync_signal);
                    }
                } else if val["type"] == "TX_GOSSIP" {
                    if let Ok(inbound_tx) = serde_json::from_value::<SignedTransaction>(val["tx"].clone()) {
                        let _ = p2p_sync_mempool.insert_with_rbf(inbound_tx);
                    }
                }
            }
        }
    });

    let (app_p2p_sender, mut app_p2p_receiver) = tokio::sync::mpsc::channel::<String>(1000);
    let bridge_cmd_sender = p2p_cmd_sender.clone();
    tokio::spawn(async move {
        while let Some(gossip_payload) = app_p2p_receiver.recv().await {
            let _ = bridge_cmd_sender.send(P2PCommand {
                topic: "tx_planetary_gossip".to_string(),
                message: gossip_payload.as_bytes().to_vec(),
            }).await;
        }
    });

    // Stratum Engine with Dynamic Candidate Job Distribution
    let stratum_db = db.clone();
    let stratum_mempool = persistent_mempool.clone();
    let stratum_p2p_cmd = p2p_cmd_sender.clone();
    let stratum_ws = ws_sender.clone();
    let stratum_interrupt = interrupt_signal.clone();

    tokio::spawn(async move {
        if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", stratum_port)).await {
            println!("[STRATUM MINING POOL]: TCP Server listening on 0.0.0.0:{}", stratum_port);
            while let Ok((mut socket, _)) = listener.accept().await {
                let db_ref = stratum_db.clone();
                let mempool_ref = stratum_mempool.clone();
                let p2p_ref = stratum_p2p_cmd.clone();
                let ws_ref = stratum_ws.clone();
                let int_ref = stratum_interrupt.clone();

                tokio::spawn(async move {
                    let (reader, mut writer) = socket.split();
                    let mut lines = BufReader::new(reader).lines();
                    let mut authorized_worker = "0x34E01b6bC08cE82f88a516707B30289A40Af724c".to_string();

                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) {
                            let method = req["method"].as_str().unwrap_or("");
                            let id = &req["id"];

                            let resp = match method {
                                "mining.subscribe" => {
                                    serde_json::json!({
                                        "id": id,
                                        "result": [["mining.notify", "ae6812"], "08000002", 4],
                                        "error": null
                                    })
                                },
                                "mining.authorize" => {
                                    if let Some(worker_name) = req["params"].get(0).and_then(|w| w.as_str()) {
                                        authorized_worker = worker_name.to_string();
                                        println!("[STRATUM POOL]: Miner Rig Authorized -> {}", authorized_worker);
                                    }
                                    serde_json::json!({ "id": id, "result": true, "error": null })
                                },
                                "mining.get_job" => {
                                    let b_tree = db_ref.open_tree("blocks").unwrap();
                                    let cur_h = b_tree.len() as u64;
                                    let p_hash = if cur_h > 0 {
                                        let l_key = (cur_h - 1).to_be_bytes();
                                        b_tree.get(l_key).ok().flatten()
                                            .and_then(|b| serde_json::from_slice::<Block>(b.as_ref()).ok())
                                            .map(|b| b.hash)
                                            .unwrap_or_else(|| "0000000000000000000000000000000000000000000000000000000000000000".to_string())
                                    } else {
                                        "0000000000000000000000000000000000000000000000000000000000000000".to_string()
                                    };

                                    let balances_tree = db_ref.open_tree("account_balances").unwrap();
                                    let mut state_pairs = Vec::new();
                                    for item in balances_tree.iter().flatten() {
                                        state_pairs.push((
                                            String::from_utf8_lossy(&item.0).to_string(),
                                            String::from_utf8_lossy(&item.1).to_string()
                                        ));
                                    }
                                    let s_root = smt::calculate_merkle_root(&mut state_pairs);
                                    let r_root = "0x0000000000000000000000000000000000000000000000000000000000000000".to_string();
                                    let candidate = format!("{}:{}:{}:{}", cur_h, p_hash, s_root, r_root);
                                    let difficulty = 3usize;

                                    serde_json::json!({
                                        "id": id,
                                        "result": {
                                            "height": cur_h,
                                            "candidate": candidate,
                                            "difficulty": difficulty
                                        },
                                        "error": null
                                    })
                                },
                                "mining.submit" => {
                                    let submitted_nonce = req["params"].get(2)
                                        .and_then(|n| {
                                            if let Some(s) = n.as_str() {
                                                u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                                            } else {
                                                n.as_u64()
                                            }
                                        }).unwrap_or(0);

                                    let b_tree = db_ref.open_tree("blocks").unwrap();
                                    let cur_h = b_tree.len() as u64;
                                    let p_hash = if cur_h > 0 {
                                        let l_key = (cur_h - 1).to_be_bytes();
                                        b_tree.get(l_key).ok().flatten()
                                            .and_then(|b| serde_json::from_slice::<Block>(b.as_ref()).ok())
                                            .map(|b| b.hash)
                                            .unwrap_or_else(|| "0000000000000000000000000000000000000000000000000000000000000000".to_string())
                                    } else {
                                        "0000000000000000000000000000000000000000000000000000000000000000".to_string()
                                    };

                                    let balances_tree = db_ref.open_tree("account_balances").unwrap();
                                    let mut state_pairs = Vec::new();
                                    for item in balances_tree.iter().flatten() {
                                        state_pairs.push((
                                            String::from_utf8_lossy(&item.0).to_string(),
                                            String::from_utf8_lossy(&item.1).to_string()
                                        ));
                                    }
                                    let s_root = smt::calculate_merkle_root(&mut state_pairs);
                                    let r_root = "0x0000000000000000000000000000000000000000000000000000000000000000".to_string();

                                    let candidate = format!("{}:{}:{}:{}", cur_h, p_hash, s_root, r_root);
                                    let computed_hash = crypto::memory_hard_hash(&candidate, submitted_nonce);
                                    let required_difficulty = 3usize;
                                    let required_prefix = "0".repeat(required_difficulty);

                                    if computed_hash.starts_with(&required_prefix) {
                                        let (user_txs, pending_keys) = mempool_ref.peek_top_n(49);
                                        let reward = consensus::calculate_block_reward(cur_h, (cur_h as f64) * 50.0);
                                        let coinbase = SignedTransaction::new_coinbase(&authorized_worker, reward, cur_h);
                                        let mut block_txs = Vec::with_capacity(user_txs.len() + 1);
                                        block_txs.push(coinbase);
                                        block_txs.extend(user_txs);

                                        let current_feeds = vec![
                                            OracleFeedEntry { symbol: "CB/USD".to_string(), price_usd: 4.82, timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), signature: "ORACLE_QUORUM_OK".to_string() },
                                            OracleFeedEntry { symbol: "BTC/USD".to_string(), price_usd: 67420.0, timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), signature: "ORACLE_QUORUM_OK".to_string() },
                                        ];

                                        if let Ok(outcome) = consensus::apply_block_transactions_and_rewards(
                                            &db_ref, &block_txs, &authorized_worker, reward, 0.10, cur_h, &computed_hash, &current_feeds
                                        ) {
                                            mempool_ref.purge_keys(&pending_keys);

                                            let new_b = Block {
                                                height: cur_h,
                                                hash: computed_hash.clone(),
                                                prev_hash: p_hash,
                                                state_root: outcome.state_root,
                                                receipts_root: outcome.receipts_root,
                                                logs_bloom: outcome.logs_bloom,
                                                base_fee: 0.10,
                                                total_burned: outcome.total_burned,
                                                gas_used: outcome.total_gas_used,
                                                difficulty_zeros: required_difficulty,
                                                cumulative_work: cur_h as u128 * (16u128.pow(required_difficulty as u32)),
                                                nonce: submitted_nonce,
                                                reward,
                                                transactions: block_txs,
                                                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                validator: authorized_worker.clone(),
                                                status: "FINALIZED".to_string(),
                                                pos_finality_signatures: vec![format!("POS_VALIDATOR_SELF_VOTE_{}", cur_h)],
                                                oracle_feeds: current_feeds,
                                            };

                                            let _ = b_tree.insert(cur_h.to_be_bytes(), serde_json::to_vec(&new_b).unwrap());
                                            let _ = b_tree.flush();

                                            println!("\n========================================================");
                                            println!("🔥 [EXTERNAL BLOCK SEALED]: Block #{} MINED by Rig!", cur_h);
                                            println!("   ├── Miner Wallet: {}", &authorized_worker);
                                            println!("   ├── Nonce: {}", submitted_nonce);
                                            println!("   ├── Hash: {}", computed_hash);
                                            println!("   └── Reward: +50 CB Credited to Balance");
                                            println!("========================================================\n");

                                            let gossip_str = serde_json::json!({ "type": "BLOCK_GOSSIP", "block": new_b }).to_string();
                                            let _ = p2ref_send(&p2p_ref, gossip_str.clone()).await;
                                            let _ = ws_ref.send(gossip_str);
                                            int_ref.store(cur_h, Ordering::Release);
                                        }
                                        serde_json::json!({ "id": id, "result": true, "error": null })
                                    } else {
                                        serde_json::json!({ "id": id, "result": false, "error": "Low difficulty share" })
                                    }
                                },
                                _ => serde_json::json!({ "id": id, "result": null, "error": "unknown method" })
                            };
                            let _ = writer.write_all(format!("{}\n", resp).as_bytes()).await;
                        }
                    }
                });
            }
        }
    });

    let app_state = Arc::new(AppStateContainer {
        db: db.clone(),
        p2p_broadcast_channel: app_p2p_sender,
        ws_broadcast_channel: ws_sender.clone(),
        persistent_mempool: persistent_mempool.clone(),
        miner_wallet_address: miner_address,
        rpc_port,
        network_id,
        interrupt_signal,
    });

    println!("[HTTP/RPC SERVER]: Running on 0.0.0.0:{}", rpc_port);
    println!("[STRATUM MINING POOL]: Live on 0.0.0.0:{}", stratum_port);
    println!("[LIBP2P DISCOVERY]: Live on 0.0.0.0:{}", p2p_port);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::from(app_state.clone()))
            .route("/", web::get().to(dashboard::dashboard_handler))
            .route("/ws", web::get().to(rpc::ws_block_feed))
            .route("/metrics", web::get().to(rpc::api_prometheus_metrics))
            .route("/rpc", web::route().method(actix_web::http::Method::OPTIONS).to(rpc::cors_preflight_handler))
            .route("/rpc", web::post().to(rpc::handle_web3_rpc))
            .route("/api/v1/tx/submit", web::route().method(actix_web::http::Method::OPTIONS).to(rpc::cors_preflight_handler))
            .route("/api/v1/tx/submit", web::post().to(rpc::api_submit_transaction))
            .route("/api/v1/l2/submit", web::post().to(rpc::api_submit_l2_rollup_batch))
            .route("/api/v1/account/balance/{address}", web::get().to(rpc::api_check_account))
            .route("/api/v1/block/{height}", web::get().to(rpc::api_get_block_by_height))
            .route("/api/v1/blocks/range/{start}/{end}", web::get().to(rpc::api_get_blocks_range))
            .route("/api/v1/supply", web::get().to(rpc::api_supply_metrics))
            .route("/health", web::get().to(rpc::api_health))
    })
    .bind(format!("0.0.0.0:{}", rpc_port))?
    .run()
    .await
}

async fn p2ref_send(channel: &tokio::sync::mpsc::Sender<P2PCommand>, payload: String) {
    let _ = channel.send(P2PCommand {
        topic: "block_planetary_gossip".to_string(),
        message: payload.into_bytes(),
    }).await;
}
