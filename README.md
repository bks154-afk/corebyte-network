# 🪐 CoreByte (CB) Sovereign Layer-1 Network

Official repository for the CoreByte high-performance sovereign Layer-1 blockchain network.

---

## ⛏️ 1. Start Mining (PoW Stratum Miner)
Connect your hardware rig to the global CoreByte Stratum pool and earn **+50 CB** coinbase block rewards.

```bash
git clone [https://github.com/bks154-afk/corebyte-network.git](https://github.com/bks154-afk/corebyte-network.git)
cd corebyte-network
cargo run --release --bin miner -- 0xYOUR_CB_WALLET_ADDRESS
Note: Replace 0xYOUR_CB_WALLET_ADDRESS with your actual payout address.

🛡️ 2. Run a Full Validator / RPC Node
Participate in consensus, host JSON-RPC APIs, and join the global Libp2p DHT mesh.

Bash
git clone [https://github.com/bks154-afk/corebyte-network.git](https://github.com/bks154-afk/corebyte-network.git)
cd corebyte-network
cargo run --release --bin block

🌐 Network Discovery & Endpoints
Service	Address / Endpoint
Master Seed / RPC Node	http://32.236.238.232:8080
Stratum Mining Pool	32.236.238.232:3333
Libp2p DHT Bootstrap	/ip4/32.236.238.232/tcp/9000
Live Metrics Dashboard	http://32.236.238.232:8080


