use actix_web::{web, HttpResponse, Responder, http::header, HttpRequest};
use actix_ws::Message;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::{OnceLock, RwLock};
use std::collections::HashSet;
use crate::types::{Block, SignedTransaction, TransactionReceipt, CBBITS_PER_CB, DEFAULT_MINER_WALLET};
use crate::mempool::PersistentMempool;
use crate::smt;
use crate::vm;
use crate::consensus::{L2_ROLLUP_SETTLEMENT_ADDRESS, record_pos_attestation};

static ROUTING_TABLE: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();

fn get_routing_table() -> &'static RwLock<HashSet<String>> {
    ROUTING_TABLE.get_or_init(|| {
        let mut set = HashSet::new();
        set.insert("127.0.0.1:8080".to_string());
        set.insert("127.0.0.1:8081".to_string());
        RwLock::new(set)
    })
}

pub struct AppStateContainer {
    pub db: sled::Db,
    pub p2p_broadcast_channel: tokio::sync::mpsc::Sender<String>,
    pub ws_broadcast_channel: tokio::sync::broadcast::Sender<String>,
    pub persistent_mempool: PersistentMempool,
    pub miner_wallet_address: String,
    pub rpc_port: u16,
    pub network_id: String,
    pub interrupt_signal: Arc<AtomicU64>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
    pub id: serde_json::Value,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub id: serde_json::Value,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct L2BatchSubmission {
    pub sequencer_address: String,
    pub batch_merkle_root: String,
    pub transaction_count: u64,
    pub compressed_state_delta: String,
    pub signature: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct PoSAttestationPayload {
    pub validator_pubkey: String,
    pub block_height: u64,
    pub block_hash: String,
    pub bls_signature: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct PeerExchangePayload {
    pub sender_node_id: String,
    pub listening_address: String,
    pub known_peers: Vec<String>,
}

pub async fn cors_preflight_handler() -> impl Responder {
    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .insert_header((header::ACCESS_CONTROL_ALLOW_HEADERS, "*"))
        .insert_header((header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"))
        .finish()
}

fn process_single_rpc(data: &web::Data<AppStateContainer>, req: &JsonRpcRequest) -> JsonRpcResponse {
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let balances_tree = data.db.open_tree("account_balances").unwrap();
    let tx_payload_tree = data.db.open_tree("tx_payloads").unwrap();
    let receipts_tree = data.db.open_tree("tx_receipts").unwrap();
    let contract_storage_tree = data.db.open_tree("contract_storage").unwrap();
    let contract_code_tree = data.db.open_tree("contract_code").unwrap();
    let current_height = blocks_tree.len();

    let result = match req.method.as_str() {
        "eth_chainId" => serde_json::json!("0x1f90"),
        "net_version" => serde_json::json!(data.rpc_port.to_string()),
        "net_listening" => serde_json::json!(true),
        "net_peerCount" => {
            let count = get_routing_table().read().unwrap().len();
            serde_json::json!(format!("0x{:x}", count))
        },
        "eth_syncing" => serde_json::json!(false),
        "eth_protocolVersion" => serde_json::json!("0x41"),
        "web3_clientVersion" => serde_json::json!("CoreByte-Layer1/v6.5.0-Complete"),
        "eth_blockNumber" => serde_json::json!(format!("0x{:x}", current_height.saturating_sub(1))),
        
        "eth_getBalance" => {
            let address = req.params.as_ref()
                .and_then(|p| p.get(0))
                .and_then(|a| a.as_str())
                .unwrap_or(DEFAULT_MINER_WALLET);

            let balance_cbbit: u128 = balances_tree.get(address.as_bytes()).ok().flatten()
                .or_else(|| balances_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
                .and_then(|v| String::from_utf8(v.to_vec()).ok())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);

            let wei_units = balance_cbbit * 10_000_000_000;
            serde_json::json!(format!("0x{:x}", wei_units))
        },

        "eth_gasPrice" => serde_json::json!("0x3b9aca00"),
        "eth_maxPriorityFeePerGas" => serde_json::json!("0x5f5e100"),
        
        "eth_estimateGas" => {
            let call_obj = req.params.as_ref().and_then(|p| p.get(0));
            let to_target = call_obj.and_then(|o| o.get("to")).and_then(|t| t.as_str()).unwrap_or("");
            let data_payload = call_obj.and_then(|o| o.get("data")).and_then(|d| d.as_str()).unwrap_or("");

            if !data_payload.is_empty() {
                let dry_run = vm::execute_bytecode("0xESTIMATE_GAS_SIMULATOR", to_target, data_payload, &contract_storage_tree);
                let estimated = (dry_run.gas_used as f64 * 1.20) as u64;
                serde_json::json!(format!("0x{:x}", estimated.max(21000)))
            } else {
                serde_json::json!("0x5208")
            }
        },

        "eth_getCode" => {
            let address = req.params.as_ref()
                .and_then(|p| p.get(0))
                .and_then(|a| a.as_str())
                .unwrap_or("0x0");

            let code_str = contract_code_tree.get(address.as_bytes()).ok().flatten()
                .or_else(|| contract_code_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
                .and_then(|v| String::from_utf8(v.to_vec()).ok())
                .unwrap_or_else(|| "0x".to_string());

            serde_json::json!(code_str)
        },

        "eth_getTransactionCount" => {
            let address = req.params.as_ref()
                .and_then(|p| p.get(0))
                .and_then(|a| a.as_str())
                .unwrap_or("0x0");

            let nonces_tree = data.db.open_tree("account_nonces").unwrap();
            let nonce: u64 = nonces_tree.get(address.as_bytes()).ok().flatten()
                .or_else(|| nonces_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
                .and_then(|v| String::from_utf8(v.to_vec()).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            serde_json::json!(format!("0x{:x}", nonce))
        },

        "eth_getTransactionByHash" => {
            let tx_hash = req.params.as_ref()
                .and_then(|p| p.get(0))
                .and_then(|a| a.as_str())
                .unwrap_or("0x0");

            if let Ok(Some(bytes)) = tx_payload_tree.get(tx_hash.as_bytes()) {
                if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(bytes.as_ref()) {
                    let tx: SignedTransaction = serde_json::from_value(meta["tx"].clone()).unwrap_or_default();
                    let b_height = meta["block_height"].as_u64().unwrap_or(0);
                    let b_hash = meta["block_hash"].as_str().unwrap_or("0x0");
                    let tx_index = meta["tx_index"].as_u64().unwrap_or(0);

                    serde_json::json!({
                        "hash": tx_hash,
                        "nonce": format!("0x{:x}", tx.nonce),
                        "blockHash": format!("0x{}", b_hash),
                        "blockNumber": format!("0x{:x}", b_height),
                        "transactionIndex": format!("0x{:x}", tx_index),
                        "from": tx.sender,
                        "to": tx.receiver,
                        "value": format!("0x{:x}", (tx.amount * 1_000_000_000_000_000_000.0) as u128),
                        "gasPrice": "0x3b9aca00",
                        "gas": "0x5208",
                        "input": tx.data.as_deref().unwrap_or("0x")
                    })
                } else {
                    serde_json::json!(null)
                }
            } else {
                serde_json::json!(null)
            }
        },

        "eth_getTransactionReceipt" => {
            let tx_hash = req.params.as_ref()
                .and_then(|p| p.get(0))
                .and_then(|a| a.as_str())
                .unwrap_or("0x0");

            if let Ok(Some(bytes)) = receipts_tree.get(tx_hash.as_bytes()) {
                if let Ok(receipt) = serde_json::from_slice::<TransactionReceipt>(bytes.as_ref()) {
                    serde_json::json!({
                        "transactionHash": receipt.transaction_hash,
                        "transactionIndex": "0x0",
                        "blockNumber": format!("0x{:x}", receipt.block_number),
                        "blockHash": receipt.block_hash,
                        "contractAddress": receipt.contract_address,
                        "cumulativeGasUsed": format!("0x{:x}", receipt.cumulative_gas_used),
                        "gasUsed": format!("0x{:x}", receipt.gas_used),
                        "status": receipt.status
                    })
                } else {
                    serde_json::json!(null)
                }
            } else {
                serde_json::json!(null)
            }
        },

        _ => serde_json::json!({ "status": "active", "chain": data.network_id }),
    };

    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        result: Some(result),
        error: None,
        id: req.id.clone(),
    }
}

pub async fn handle_web3_rpc(data: web::Data<AppStateContainer>, payload: web::Bytes) -> impl Responder {
    if let Ok(single_req) = serde_json::from_slice::<JsonRpcRequest>(&payload) {
        let resp = process_single_rpc(&data, &single_req);
        HttpResponse::Ok()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(resp)
    } else if let Ok(batch_reqs) = serde_json::from_slice::<Vec<JsonRpcRequest>>(&payload) {
        let mut responses = Vec::with_capacity(batch_reqs.len());
        for req in &batch_reqs {
            responses.push(process_single_rpc(&data, req));
        }
        HttpResponse::Ok()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(responses)
    } else {
        HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({
                "jsonrpc": "2.0",
                "error": { "code": -32700, "message": "Parse error: Invalid JSON-RPC payload" },
                "id": null
            }))
    }
}

pub async fn ws_block_feed(
    req: HttpRequest,
    stream: web::Payload,
    data: web::Data<AppStateContainer>,
) -> Result<HttpResponse, actix_web::Error> {
    let (res, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;
    let mut rx = data.ws_broadcast_channel.subscribe();

    actix_web::rt::spawn(async move {
        loop {
            tokio::select! {
                Ok(msg) = rx.recv() => {
                    let wrapped_event = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "eth_subscription",
                        "params": {
                            "subscription": "0x1",
                            "result": serde_json::from_str::<serde_json::Value>(&msg).unwrap_or(serde_json::json!(msg))
                        }
                    }).to_string();

                    if session.text(wrapped_event).await.is_err() {
                        break;
                    }
                }
                Some(Ok(msg)) = msg_stream.next() => {
                    match msg {
                        Message::Ping(bytes) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                else => break,
            }
        }
    });

    Ok(res)
}

pub async fn api_submit_transaction(
    data: web::Data<AppStateContainer>,
    req: web::Json<SignedTransaction>,
) -> impl Responder {
    let tx = req.into_inner();

    if !tx.verify_signature() {
        return HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({ "error": "Signature rejected: ECDSA Secp256k1 validation failure" }));
    }

    let nonces_tree = data.db.open_tree("account_nonces").unwrap();
    let current_nonce: u64 = nonces_tree.get(tx.sender.as_bytes()).ok().flatten()
        .or_else(|| nonces_tree.get(tx.sender.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if tx.nonce <= current_nonce || tx.nonce > current_nonce + 64 {
        return HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({
                "error": format!("Nonce rejected: expected contiguous nonce > {}, got {}.", current_nonce, tx.nonce)
            }));
    }

    let balances_tree = data.db.open_tree("account_balances").unwrap();
    let balance_cbbit: u128 = balances_tree.get(tx.sender.as_bytes()).ok().flatten()
        .or_else(|| balances_tree.get(tx.sender.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0);

    let required_cbbit = ((tx.amount + tx.priority_fee) * CBBITS_PER_CB as f64).round() as u128;
    if balance_cbbit < required_cbbit {
        return HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({
                "error": format!("Insufficient balance: requires {} CBbit, found {} CBbit", required_cbbit, balance_cbbit)
            }));
    }

    let tx_digest = tx.message_digest();
    match data.persistent_mempool.insert_with_rbf(tx.clone()) {
        Ok(tx_hash) => {
            let gossip_packet = serde_json::json!({
                "type": "TX_GOSSIP",
                "tx": tx
            }).to_string();
            let _ = data.p2p_broadcast_channel.send(gossip_packet).await;

            HttpResponse::Ok()
                .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
                .json(serde_json::json!({
                    "status": "Transaction safely persisted in Sled Mempool",
                    "tx_hash": tx_hash,
                    "tx_digest": tx_digest,
                    "fee_priority_cb": tx.priority_fee,
                    "pending_mempool_size": data.persistent_mempool.count()
                }))
        }
        Err(err) => HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({ "error": err })),
    }
}

pub async fn api_submit_l2_rollup_batch(
    data: web::Data<AppStateContainer>,
    req: web::Json<L2BatchSubmission>,
) -> impl Responder {
    let batch = req.into_inner();
    let nonces_tree = data.db.open_tree("account_nonces").unwrap();
    let current_nonce: u64 = nonces_tree.get(batch.sequencer_address.as_bytes()).ok().flatten()
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let settlement_calldata = format!(
        "BATCH_ROOT:{}:TX_COUNT:{}:DELTA:{}",
        batch.batch_merkle_root, batch.transaction_count, batch.compressed_state_delta
    );

    let l1_settlement_tx = SignedTransaction {
        sender: batch.sequencer_address,
        receiver: L2_ROLLUP_SETTLEMENT_ADDRESS.to_string(),
        amount: 0.0,
        priority_fee: 0.05,
        nonce: current_nonce + 1,
        public_key: "0x34E01b6bC08cE82f88a516707B30289A40Af724c".to_string(),
        signature: batch.signature,
        chain_id: 8080,
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        data: Some(settlement_calldata),
        paymaster: None,
        paymaster_signature: None,
    };

    match data.persistent_mempool.insert_with_rbf(l1_settlement_tx.clone()) {
        Ok(tx_hash) => HttpResponse::Ok()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({
                "status": "L2 Rollup Batch Accepted for L1 Finality Anchor",
                "l1_settlement_hash": tx_hash,
                "batch_root": batch.batch_merkle_root,
                "virtual_tps_multiplier": batch.transaction_count,
            })),
        Err(e) => HttpResponse::BadRequest()
            .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .json(serde_json::json!({ "error": e })),
    }
}

pub async fn api_submit_pos_attestation(
    req: web::Json<PoSAttestationPayload>,
) -> impl Responder {
    let payload = req.into_inner();
    let attestation_sig = format!("{}:{}:{}", payload.validator_pubkey, payload.block_hash, payload.bls_signature);
    record_pos_attestation(payload.block_height, attestation_sig);

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "status": "PoS Attestation Registered in Memory Pool",
            "block_height": payload.block_height,
            "validator": payload.validator_pubkey
        }))
}

pub async fn api_peer_exchange(
    req: web::Json<PeerExchangePayload>,
) -> impl Responder {
    let payload = req.into_inner();
    let mut table = get_routing_table().write().unwrap();
    table.insert(payload.listening_address);
    for peer in payload.known_peers {
        table.insert(peer);
    }

    let active_peers: Vec<String> = table.iter().cloned().collect();
    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "status": "Peer Table Synchronized",
            "routing_table_size": active_peers.len(),
            "active_peers": active_peers
        }))
}

pub async fn api_check_account(data: web::Data<AppStateContainer>, path: web::Path<String>) -> impl Responder {
    let address = path.into_inner();
    let balances_tree = data.db.open_tree("account_balances").unwrap();
    let nonces_tree = data.db.open_tree("account_nonces").unwrap();

    let balance_cbbit: u128 = balances_tree.get(address.as_bytes()).ok().flatten()
        .or_else(|| balances_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0);

    let nonce: u64 = nonces_tree.get(address.as_bytes()).ok().flatten()
        .or_else(|| nonces_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let shard_id = if let Some(hex_byte) = address.strip_prefix("0x").and_then(|s| s.get(0..2)) {
        u8::from_str_radix(hex_byte, 16).unwrap_or(0) % 4
    } else {
        0
    };

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "address": address,
            "balance_cb": balance_cbbit as f64 / CBBITS_PER_CB as f64,
            "balance_cbbit": balance_cbbit,
            "nonce": nonce,
            "routed_shard_id": shard_id,
            "execution_mode": "Conflict-Free Parallel Lane"
        }))
}

pub async fn api_get_account_proof(data: web::Data<AppStateContainer>, path: web::Path<String>) -> impl Responder {
    let address = path.into_inner();
    let balances_tree = data.db.open_tree("account_balances").unwrap();
    let nonces_tree = data.db.open_tree("account_nonces").unwrap();

    let balance_cbbit: u128 = balances_tree.get(address.as_bytes()).ok().flatten()
        .or_else(|| balances_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0);

    let nonce: u64 = nonces_tree.get(address.as_bytes()).ok().flatten()
        .or_else(|| nonces_tree.get(address.to_lowercase().as_bytes()).ok().flatten())
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut state_pairs = Vec::new();
    for item in balances_tree.iter().flatten() {
        state_pairs.push((
            String::from_utf8_lossy(&item.0).to_string(),
            String::from_utf8_lossy(&item.1).to_string()
        ));
    }

    let proof_nodes = smt::generate_merkle_proof(&mut state_pairs, &address);
    let state_root = smt::calculate_merkle_root(&mut state_pairs);

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "address": address,
            "balance_cb": balance_cbbit as f64 / CBBITS_PER_CB as f64,
            "balance_cbbit": balance_cbbit,
            "nonce": nonce,
            "state_root": state_root,
            "account_proof": proof_nodes
        }))
}

pub async fn api_export_state_snapshot(data: web::Data<AppStateContainer>) -> impl Responder {
    let balances_tree = data.db.open_tree("account_balances").unwrap();
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let height = blocks_tree.len() as u64;

    let mut accounts = Vec::new();
    let mut state_pairs = Vec::new();
    for item in balances_tree.iter().flatten() {
        let addr = String::from_utf8_lossy(&item.0).to_string();
        let bal = String::from_utf8_lossy(&item.1).to_string();
        accounts.push(serde_json::json!({ "address": addr, "balance_cbbit": bal }));
        state_pairs.push((addr, bal));
    }

    let state_root = smt::calculate_merkle_root(&mut state_pairs);

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "block_height": height,
            "state_root": state_root,
            "total_accounts": accounts.len(),
            "ledger_snapshot": accounts
        }))
}

pub async fn api_get_contract_storage(
    data: web::Data<AppStateContainer>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (contract, key) = path.into_inner();
    let storage_tree = data.db.open_tree("contract_storage").unwrap();
    let storage_key = format!("{}:{}", contract, key);

    let val = storage_tree.get(storage_key.as_bytes()).ok().flatten()
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .unwrap_or_else(|| "0".to_string());

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "contract": contract,
            "key": key,
            "value": val
        }))
}

pub async fn api_get_block_by_height(data: web::Data<AppStateContainer>, path: web::Path<u64>) -> impl Responder {
    let height = path.into_inner();
    let blocks_tree = data.db.open_tree("blocks").unwrap();

    if let Ok(Some(bytes)) = blocks_tree.get(height.to_be_bytes()) {
        if let Ok(block) = serde_json::from_slice::<Block>(bytes.as_ref()) {
            return HttpResponse::Ok()
                .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
                .json(block);
        }
    }

    HttpResponse::NotFound()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({ "error": format!("Block #{} not found on mainnet ledger", height) }))
}

pub async fn api_get_blocks_range(data: web::Data<AppStateContainer>, path: web::Path<(u64, u64)>) -> impl Responder {
    let (start, end) = path.into_inner();
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let mut blocks = Vec::new();

    let from_h = start.min(end);
    let to_h = start.max(end);

    for h in from_h..=to_h {
        if let Ok(Some(bytes)) = blocks_tree.get(h.to_be_bytes()) {
            if let Ok(b) = serde_json::from_slice::<Block>(bytes.as_ref()) {
                blocks.push(b);
            }
        }
    }

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(blocks)
}

pub async fn api_peer_handshake(data: web::Data<AppStateContainer>) -> impl Responder {
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let height = blocks_tree.len() as u64;
    let genesis_key = 0u64.to_be_bytes();
    let genesis_hash = blocks_tree.get(genesis_key).ok().flatten()
        .and_then(|bytes| serde_json::from_slice::<Block>(bytes.as_ref()).ok())
        .map(|b| b.hash)
        .unwrap_or_else(|| "0x0".to_string());

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "network": data.network_id,
            "chain_id": 8080,
            "client_version": "CoreByte-Layer1/v6.5.0-Complete",
            "current_height": height,
            "genesis_block_hash": genesis_hash,
            "active_kademlia_peers": get_routing_table().read().unwrap().len(),
            "p2p_active": true
        }))
}

pub async fn api_peer_discover(data: web::Data<AppStateContainer>) -> impl Responder {
    let active_peers: Vec<String> = get_routing_table().read().unwrap().iter().cloned().collect();
    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "self_node_id": format!("node_{:x}", data.rpc_port),
            "network": data.network_id,
            "active_peers": active_peers,
            "protocol_version": 2
        }))
}

pub async fn api_supply_metrics(data: web::Data<AppStateContainer>) -> impl Responder {
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let metrics_tree = data.db.open_tree("global_metrics").unwrap();
    let l2_batches_tree = data.db.open_tree("l2_settled_batches").unwrap();
    let height = blocks_tree.len() as u64;

    let total_burned: f64 = metrics_tree.get(b"total_burned").ok().flatten()
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let gross_minted = (height as f64) * 50.0;
    let net_circulating = (gross_minted - total_burned).max(0.0);
    let next_halving = ((height / 210_000) + 1) * 210_000;
    let blocks_until_halving = next_halving.saturating_sub(height);

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "token_symbol": "CB",
            "fractional_unit": "CBbit",
            "conversion": "1 CB = 100,000,000 CBbit",
            "max_supply_cb": 21_000_000.0,
            "gross_minted_cb": gross_minted,
            "total_burned_cb": total_burned,
            "circulating_supply_cb": net_circulating,
            "circulating_supply_cbbit": (net_circulating * CBBITS_PER_CB as f64).round() as u128,
            "burned_percentage": (total_burned / gross_minted.max(1.0)) * 100.0,
            "current_block_height": height.saturating_sub(1),
            "next_halving_block": next_halving,
            "blocks_until_halving": blocks_until_halving,
            "l2_settled_batches_count": l2_batches_tree.len(),
            "rolling_pruning_enabled": true,
            "kademlia_routing_mesh": true
        }))
}

pub async fn api_prometheus_metrics(data: web::Data<AppStateContainer>) -> impl Responder {
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let metrics_tree = data.db.open_tree("global_metrics").unwrap();
    let l2_batches_tree = data.db.open_tree("l2_settled_batches").unwrap();
    let height = blocks_tree.len();
    let mempool_size = data.persistent_mempool.count();
    let threads = num_cpus::get();

    let total_burned: f64 = metrics_tree.get(b"total_burned").ok().flatten()
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let metrics_text = format!(
        "# HELP cb_block_height Current height of the finalized CoreByte blockchain\n\
         # TYPE cb_block_height counter\n\
         cb_block_height {}\n\n\
         # HELP cb_mempool_queued Current pending unmined transactions\n\
         # TYPE cb_mempool_queued gauge\n\
         cb_mempool_queued {}\n\n\
         # HELP cb_routing_peers Active peers in Kademlia routing mesh\n\
         # TYPE cb_routing_peers gauge\n\
         cb_routing_peers {}\n\n\
         # HELP cb_l2_batches_settled Total Layer-2 rollups anchored\n\
         # TYPE cb_l2_batches_settled counter\n\
         cb_l2_batches_settled {}\n\n\
         # HELP cb_mining_threads Active parallel mining CPU threads\n\
         # TYPE cb_mining_threads gauge\n\
         cb_mining_threads {}\n\n\
         # HELP cb_total_burned_supply_cb Total CoreByte permanently burned\n\
         # TYPE cb_total_burned_supply_cb counter\n\
         cb_total_burned_supply_cb {:.6}\n\n\
         # HELP cb_network_active 1 if node is live\n\
         # TYPE cb_network_active gauge\n\
         cb_network_active 1\n",
        height, mempool_size, get_routing_table().read().unwrap().len(), l2_batches_tree.len(), threads, total_burned
    );

    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(metrics_text)
}

pub async fn api_health(data: web::Data<AppStateContainer>) -> impl Responder {
    let blocks_tree = data.db.open_tree("blocks").unwrap();
    let metrics_tree = data.db.open_tree("global_metrics").unwrap();
    let height = blocks_tree.len() as f64;

    let total_burned: f64 = metrics_tree.get(b"total_burned").ok().flatten()
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let gross_minted = height * 50.0;
    let net_circulating = (gross_minted - total_burned).max(0.0);

    HttpResponse::Ok()
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(serde_json::json!({
            "status": "HEALTHY",
            "chain_height": blocks_tree.len(),
            "token_symbol": "CB",
            "fractional_unit": "CBbit",
            "mempool_queued": data.persistent_mempool.count(),
            "cpu_parallel_threads": num_cpus::get(),
            "consensus": "PoW + PoS Dual-Consensus Engine",
            "zk_snark_precompile": "Alt-BN128 Bilinear Pairing Enabled",
            "state_pruning": "Rolling Horizon Active (10,000 blocks)",
            "p2p_mesh": "Kademlia Multi-Hop Dynamic Table",
            "max_supply_cap": "21,000,000 CB",
            "total_burned_supply_cb": total_burned,
            "circulating_supply_cb": net_circulating,
            "p2p_gossip_active": true,
            "websocket_active": true
        }))
}