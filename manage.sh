#!/bin/bash

BASE_DIR="/root/corebyte"
cd "$BASE_DIR" || exit 1

TREASURY_ADDR="0x34E01b6bC08cE82f88a516707B30289A40Af724c"
PERSONAL_ADDR="0x34E01b6bC08cE82f88a516707B30289A40Af724c"

show_menu() {
    clear
    echo "===================================================================="
    echo "       🪐 COREBYTE (CB) SERVER MASTER CONTROL PANEL"
    echo "===================================================================="
    echo "1. 🔍 Full System & Network Audit (Rust Monitor Run)"
    echo "2. ⛏️ Active Connected Miner IPs & Connections Check"
    echo "3. 🪙 Active Authorized Mining Wallets Check"
    echo "4. 🏛️ Check Official Protocol Treasury Balance (R&D / Validators)"
    echo "5. 👤 Check My Personal Private Wallet Balance"
    echo "6. 📜 View Live Blockchain Node Logs (journalctl)"
    echo "7. 🔄 Restart CoreByte Node Service"
    echo "8. 🛑 Stop CoreByte Node Service"
    echo "9. ▶️ Start CoreByte Node Service"
    echo "10. 🛠️ Recompile Node Binary (Release Mode)"
    echo "0. ❌ Exit"
    echo "===================================================================="
    echo -n "Option select karein [0-10]: "
}

while true; do
    show_menu
    read -r choice
    case $choice in
        1)
            echo -e "\n[*] Running Rust System Monitor..."
            cargo run --release --bin monitor
            read -p "Press Enter to continue..."
            ;;
        2)
            echo -e "\n[*] Live Connected Miner IPs on Port 3333:"
            ss -tn state established '( sport = :3333 )' | awk '{print $4}' | cut -d: -f1 | grep -v "127.0.0.1" | grep -v "Address" | sort | uniq -c
            read -p "Press Enter to continue..."
            ;;
        3)
            echo -e "\n[*] Authorized Mining Wallets (Last 500 lines):"
            journalctl -u corebyte.service -n 500 --no-pager | grep "Miner Rig Authorized" | awk '{print $NF}' | sort -u
            read -p "Press Enter to continue..."
            ;;
        4)
            echo -e "\n[*] Fetching Official Protocol Treasury Balance:"
            echo -e "    Address: $TREASURY_ADDR"
            python3 -c "import urllib.request, json; data = json.loads(urllib.request.urlopen('http://127.0.0.1:8080/api/v1/account/balance/$TREASURY_ADDR').read()); print('    ├── Protocol Treasury Balance:', f'{data.get(\"balance_cb\", 0):,.2f}', 'CB')"
            read -p "Press Enter to continue..."
            ;;
        5)
            echo -e "\n[*] Fetching Your Personal Private Wallet Balance:"
            echo -e "    Address: $PERSONAL_ADDR"
            python3 -c "import urllib.request, json; data = json.loads(urllib.request.urlopen('http://127.0.0.1:8080/api/v1/account/balance/$PERSONAL_ADDR').read()); print('    ├── Personal Wallet Balance:', f'{data.get(\"balance_cb\", 0):,.2f}', 'CB')"
            read -p "Press Enter to continue..."
            ;;
        6)
            echo -e "\n[*] Showing Live Logs (Ctrl+C dabayein bahar aane ke liye)..."
            journalctl -u corebyte.service -f --no-pager
            ;;
        7)
            echo -e "\n[*] Restarting corebyte.service..."
            systemctl restart corebyte.service
            sleep 2
            systemctl status corebyte.service --no-pager
            read -p "Press Enter to continue..."
            ;;
        8)
            echo -e "\n[*] Stopping corebyte.service..."
            systemctl stop corebyte.service
            read -p "Press Enter to continue..."
            ;;
        9)
            echo -e "\n[*] Starting corebyte.service..."
            systemctl start corebyte.service
            read -p "Press Enter to continue..."
            ;;
        10)
            echo -e "\n[*] Compiling CoreByte binaries..."
            cargo build --release
            read -p "Press Enter to continue..."
            ;;
        0)
            echo -e "\nGoodbye!"
            exit 0
            ;;
        *)
            echo -e "\nInvalid option!"
            sleep 1
            ;;
    esac
done
