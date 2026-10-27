use serde_json::json;
use sha2::{Digest, Sha256, Sha512};
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_POOL_HOST: &str = "127.0.0.1";
const DEFAULT_POOL_PORT: u16 = 3333;
const DEFAULT_WALLET: &str = "0x34E01b6bC08cE82f88a516707B30289A40Af724c";

fn hash_quantum_secure(input: &str) -> String {
    let mut hasher1 = Sha512::new();
    hasher1.update(input.as_bytes());
    let round1 = hasher1.finalize();

    let mut hasher2 = Sha512::new();
    hasher2.update(round1);
    format!("{:x}", hasher2.finalize())
}

fn memory_hard_hash(candidate: &str, nonce: u64) -> String {
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

fn main() {
    let args: Vec<String> = env::args().collect();
    let wallet_addr = if args.len() > 1 {
        args[1].clone()
    } else {
        DEFAULT_WALLET.to_string()
    };

    println!("====================================================================");
    println!("🪐 CoreByte (CB) OFFICIAL STRATUM POW MINER (v1.0.0)");
    println!("⚡ Target: {}:{}", DEFAULT_POOL_HOST, DEFAULT_POOL_PORT);
    println!("🪙 Payout Address: {}", wallet_addr);
    println!("⚙️ Parallel Threads: {}", num_cpus::get().max(1));
    println!("====================================================================");

    let num_threads = num_cpus::get().max(1);

    loop {
        match TcpStream::connect(format!("{}:{}", DEFAULT_POOL_HOST, DEFAULT_POOL_PORT)) {
            Ok(mut stream) => {
                let stream_clone = match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => { thread::sleep(Duration::from_secs(2)); continue; }
                };

                let mut reader = BufReader::new(stream_clone);

                let sub_payload = json!({
                    "id": 1,
                    "method": "mining.subscribe",
                    "params": []
                }).to_string() + "\n";
                if stream.write_all(sub_payload.as_bytes()).is_err() { continue; }
                let mut sub_resp = String::new();
                let _ = reader.read_line(&mut sub_resp);

                let auth_payload = json!({
                    "id": 2,
                    "method": "mining.authorize",
                    "params": [&wallet_addr, "x"]
                }).to_string() + "\n";
                if stream.write_all(auth_payload.as_bytes()).is_err() { continue; }
                let mut auth_resp = String::new();
                let _ = reader.read_line(&mut auth_resp);

                let job_payload = json!({
                    "id": 3,
                    "method": "mining.get_job",
                    "params": []
                }).to_string() + "\n";
                if stream.write_all(job_payload.as_bytes()).is_err() { continue; }
                let mut job_resp = String::new();
                let _ = reader.read_line(&mut job_resp);

                let job_val: serde_json::Value = serde_json::from_str(&job_resp).unwrap_or(json!({}));
                let candidate = job_val["result"]["candidate"].as_str().unwrap_or("").to_string();
                let target_difficulty = job_val["result"]["difficulty"].as_u64().unwrap_or(3) as usize;
                let target_height = job_val["result"]["height"].as_u64().unwrap_or(0);

                if candidate.is_empty() {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }

                println!("\n[*] Mining Job Synced | Height: #{} | Target: {} zeros", target_height, target_difficulty);
                println!("    └── Candidate: {}", &candidate[..candidate.len().min(45)]);

                let target_prefix = "0".repeat(target_difficulty);
                let found_flag = Arc::new(AtomicBool::new(false));
                let total_hashes = Arc::new(AtomicU64::new(0));
                let found_nonce = Arc::new(AtomicU64::new(0));
                let start_time = Instant::now();

                let mut handles = vec![];
                let base_nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;

                for thread_id in 0..num_threads {
                    let flag = found_flag.clone();
                    let counter = total_hashes.clone();
                    let nonce_out = found_nonce.clone();
                    let prefix = target_prefix.clone();
                    let cand = candidate.clone();

                    let handle = thread::spawn(move || {
                        let mut local_nonce = base_nonce.wrapping_add((thread_id as u64).wrapping_mul(10_000_000));
                        let stride = num_threads as u64;

                        while !flag.load(Ordering::Relaxed) {
                            let hash = memory_hard_hash(&cand, local_nonce);

                            if hash.starts_with(&prefix) {
                                flag.store(true, Ordering::SeqCst);
                                nonce_out.store(local_nonce, Ordering::SeqCst);
                                break;
                            }

                            local_nonce = local_nonce.wrapping_add(stride);
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                    handles.push(handle);
                }

                let monitor_flag = found_flag.clone();
                let monitor_counter = total_hashes.clone();
                let monitor_handle = thread::spawn(move || {
                    while !monitor_flag.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(1500));
                        let hashes = monitor_counter.load(Ordering::Relaxed);
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let khs = if elapsed > 0.0 { (hashes as f64 / elapsed) / 1000.0 } else { 0.0 };
                        print!("\r[*] Nonces Computed: {} | Speed: {:.2} kH/s (Cores: {})", hashes, khs, num_threads);
                        let _ = std::io::stdout().flush();
                    }
                });

                for h in handles { let _ = h.join(); }
                let _ = monitor_handle.join();

                let solved_nonce = found_nonce.load(Ordering::SeqCst);
                let final_hashes = total_hashes.load(Ordering::Relaxed);
                let elapsed = start_time.elapsed().as_secs_f64();
                let khs = if elapsed > 0.0 { (final_hashes as f64 / elapsed) / 1000.0 } else { 0.0 };

                println!("\n\n[🎉 SUCCESS] Valid PoW Proof Found!");
                println!("    ├── Solved Nonce: {}", solved_nonce);
                println!("    ├── Hashrate: {:.2} kH/s", khs);
                println!("    └── Submitting share to CoreByte Validator...");

                let submit_payload = json!({
                    "id": 4,
                    "method": "mining.submit",
                    "params": [&wallet_addr, "job1", format!("0x{:x}", solved_nonce)]
                }).to_string() + "\n";

                if stream.write_all(submit_payload.as_bytes()).is_ok() {
                    let mut submit_resp = String::new();
                    let _ = reader.read_line(&mut submit_resp);
                    println!("[POOL RESPONSE]: {}", submit_resp.trim());
                }

                thread::sleep(Duration::from_millis(1500));
            }
            Err(e) => {
                eprintln!("[!] Connection failed: {:?}. Retrying in 5s...", e);
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}
