# Validator Guide — Unauthority (LOS) v2.2.0

This guide covers everything you need to run a validator node on the Unauthority network: installation, Tor setup, configuration, registration, rewards, monitoring, and maintenance.

---

## Table of Contents

1. [Requirements](#requirements)
2. [Installation](#installation)
3. [Tor Hidden Service Setup](#tor-hidden-service-setup)
4. [Running the Node](#running-the-node)
5. [Registering as a Validator](#registering-as-a-validator)
6. [Systemd Service (Production)](#systemd-service-production)
7. [Configuration Reference](#configuration-reference)
8. [Validator Rewards](#validator-rewards)
9. [Slashing & Penalties](#slashing--penalties)
10. [Monitoring](#monitoring)
11. [Maintenance & Upgrades](#maintenance--upgrades)
12. [Troubleshooting](#troubleshooting)

---

## Requirements

| Component | Minimum | Recommended |
|---|---|---|
| **OS** | Linux (Ubuntu 22.04+) / macOS | Ubuntu 24.04 LTS |
| **Rust** | 1.75+ | Latest stable |
| **CPU** | 2 cores | 4 cores |
| **RAM** | 2 GB | 4 GB |
| **Disk** | 10 GB SSD | 50 GB SSD |
| **Tor** | Installed and running | Latest stable |
| **Stake** | 1 LOS (register) / 1,000 LOS (rewards) | 1,000+ LOS |
| **Uptime** | ≥95% for rewards | 99%+ |
| **Network** | Any internet connection | Stable, low latency |

---

## Installation

### Quick Install (Recommended)

```bash
# Clone the repository
git clone https://github.com/monkey-king-code/unauthority-core.git
cd unauthority-core

# Build for mainnet
./install.sh --mainnet
```

The `install.sh` script will:
1. Check that Rust is installed (prompts to install if not)
2. Check that Tor is installed and SOCKS5 proxy is reachable
3. Build the `los-node` binary with mainnet features
4. Display the binary location

### Manual Build

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install Tor
sudo apt install -y tor          # Ubuntu/Debian
brew install tor                  # macOS

# Build mainnet binary
cargo build --release -p los-node --features mainnet

# Binary location
ls -la target/release/los-node
```

### Install Tor

**Ubuntu/Debian:**
```bash
sudo apt install -y tor
sudo systemctl enable --now tor
```

**macOS (Homebrew):**
```bash
brew install tor
brew services start tor
```

**Verify Tor is running:**
```bash
# Check SOCKS5 proxy
curl --socks5-hostname 127.0.0.1:9050 https://check.torproject.org/api/ip
```

---

## Tor Hidden Service Setup

Each validator MUST have its own unique `.onion` address to participate in the network. This is how other nodes identify and reach your validator.

### Step 1: Configure Tor Hidden Service

Edit the Tor configuration file:

**Linux:** `/etc/tor/torrc`  
**macOS (Homebrew):** `/opt/homebrew/etc/tor/torrc`

Add these lines:

```
# Unauthority Validator Hidden Service
HiddenServiceDir /var/lib/tor/los-validator/
HiddenServicePort 3030 127.0.0.1:3030    # REST API
HiddenServicePort 4030 127.0.0.1:4030    # P2P Gossip
HiddenServicePort 23030 127.0.0.1:23030  # gRPC
```

> **Important:** The port numbers must match your node's `--port` flag. If you use `--port 3030`, then:
> - REST API = 3030
> - P2P Gossip = 4030 (REST + 1000)
> - gRPC = 23030 (REST + 20000)

### Step 2: Restart Tor

```bash
sudo systemctl restart tor       # Linux
brew services restart tor         # macOS
```

### Step 3: Get Your .onion Address

```bash
sudo cat /var/lib/tor/los-validator/hostname
# Example output: abc123def456ghi789jkl012mno345pqr678stu901vwxyz.onion
```

Save this address — it is your validator's network identity.

### Step 4: Set Environment Variable (Optional)

The node can auto-read the `.onion` address from the Tor hidden service directory. If auto-detection doesn't work, set it manually:

```bash
export LOS_ONION_ADDRESS=$(sudo cat /var/lib/tor/los-validator/hostname)
```

---

## Running the Node

### Minimal Start (Auto-Bootstrap)

```bash
export LOS_WALLET_PASSWORD='your-strong-password'
./target/release/los-node --port 3030 --data-dir /opt/los-node
```

The node will:
1. **Auto-discover bootstrap peers** from `genesis_config.json` (4 genesis validators with `.onion` addresses)
2. **Auto-detect Tor SOCKS5** proxy at `127.0.0.1:9050`
3. **Create a wallet** on first run — a Dilithium5 post-quantum keypair encrypted with your password
4. **Connect to peers** and begin syncing the ledger
5. **Participate in consensus** once synced and registered

### Full Control (Manual Overrides)

```bash
export LOS_WALLET_PASSWORD='your-strong-password'
export LOS_ONION_ADDRESS='your-onion-address.onion'
export LOS_SOCKS5_PROXY='socks5h://127.0.0.1:9050'
export LOS_BOOTSTRAP_NODES='peer1.onion:4030,peer2.onion:4031'
export LOS_NODE_ID='my-validator'

./target/release/los-node \
  --port 3030 \
  --data-dir /opt/los-node \
  --node-id my-validator
```

### First Run Output

On first start, you'll see:
```
🔑 New wallet created: LOSW...
🧅 Auto-detected Tor SOCKS5 proxy at 127.0.0.1:9050
🌐 Bootstrapping from 4 genesis validators...
✅ Connected to 4 peers
📊 Genesis loaded: 8 accounts, supply 21,936,236 LOS
```

---

## Registering as a Validator

After your node is running and your account has ≥1 LOS (≥1,000 LOS for reward eligibility):

### Via REST API

```bash
curl -X POST http://localhost:3030/register-validator \
  -H "Content-Type: application/json" \
  -d '{
    "address": "YOUR_LOS_ADDRESS",
    "public_key": "HEX_DILITHIUM5_PUBLIC_KEY",
    "signature": "HEX_SIGNATURE_OF_REGISTER_PAYLOAD",
    "endpoint": "your-onion-address.onion:3030"
  }'
```

The signature must be a Dilithium5 signature over the registration payload. The registration is gossiped to all peers and takes effect immediately.

### Via CLI

```bash
los-cli validator register --endpoint your-onion.onion:3030
```

> **Note:** The CLI reads your wallet keypair from the data directory and signs the registration payload automatically.

### Check Your Registration

```bash
curl http://localhost:3030/validators | python3 -m json.tool
```

---

## Systemd Service (Production)

For production deployments, run the node as a systemd service:

### Step 1: Create Dedicated User

For security, run the validator as a dedicated non-root user:

```bash
sudo useradd -r -m -d /opt/los-node -s /usr/sbin/nologin los
sudo chown -R los:los /opt/los-node
```

### Step 2: Create Service File

```bash
sudo tee /etc/systemd/system/los-node.service << 'EOF'
[Unit]
Description=Unauthority (LOS) Validator Node
After=network-online.target tor.service
Wants=network-online.target
Requires=tor.service

[Service]
Type=simple
User=los
Group=los
WorkingDirectory=/opt/unauthority-core
Environment=LOS_WALLET_PASSWORD=your-strong-password
Environment=LOS_NODE_ID=my-validator
ExecStart=/usr/local/bin/los-node --port 3030 --data-dir /opt/los-node
Restart=always
RestartSec=10
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
```

### Step 3: Copy Binary & Enable Service

```bash
# Copy binary to system path
sudo cp target/release/los-node /usr/local/bin/los-node

# Reload systemd and enable
sudo systemctl daemon-reload
sudo systemctl enable --now los-node

# Check status
sudo systemctl status los-node
```

### Step 4: View Logs

```bash
# Follow logs in real-time
journalctl -u los-node -f

# Last 100 lines
journalctl -u los-node -n 100 --no-pager
```

---

## Configuration Reference

### Environment Variables (Complete)

| Variable | Required | Default | Description |
|---|---|---|---|
| `LOS_WALLET_PASSWORD` | **Yes (mainnet)** | — | Password to encrypt/decrypt wallet keypair |
| `LOS_ONION_ADDRESS` | No | Auto from Tor dir | This node's `.onion` address for network identity |
| `LOS_SOCKS5_PROXY` | No | Auto `127.0.0.1:9050` | Tor SOCKS5 proxy for outbound connections |
| `LOS_BOOTSTRAP_NODES` | No | Auto from genesis | Comma-separated `host:port` list of bootstrap peers |
| `LOS_NODE_ID` | No | `node-{port}` | Human-readable node identifier for logs |
| `LOS_P2P_PORT` | No | REST + 1000 | P2P gossip listen port |
| `LOS_BIND_ALL` | No | `0` | Set to `1` to bind `0.0.0.0` (not recommended) |
| `LOS_TESTNET_LEVEL` | No | `consensus` | Testnet mode: `functional` / `consensus` / `production` |

### CLI Flags

| Flag | Description | Default |
|---|---|---|
| `--port <PORT>` | REST API listen port | `3030` |
| `--data-dir <DIR>` | Data directory for ledger, wallet, checkpoints | `node_data/node-{port}/` |
| `--node-id <ID>` | Node identifier | `node-{port}` |
| `--mine` | Enable PoW mining (background thread) | off |
| `--mine-threads <N>` | Number of mining threads | `1` |
| `--json-log` | Output logs as JSON (for Flutter dashboard parsing) | off |
| `--config <FILE>` | Load additional config from TOML file | none |

### Port Derivation

Given `--port 3030`:

| Service | Port | How |
|---|---|---|
| REST API | 3030 | `--port` value |
| P2P Gossip | 4030 | `--port` + 1000 |
| gRPC | 23030 | `--port` + 20000 |

### Configuration File (`validator.toml`)

When using `--config validator.toml`, the node loads additional settings from a TOML file. A reference `validator.toml` is included in the repository root. Key sections:

```toml
[validator]
node_id = "validator-1"
address = "${LOS_VALIDATOR_ADDRESS}"
private_key_path = "${LOS_VALIDATOR_PRIVKEY_PATH}"
stake_cil = 100000000000000  # 1,000 LOS
auto_claim_rewards = true

[sentry_public]
listen_addr = "0.0.0.0"
listen_port = 30333
external_addr = "${LOS_ONION_ADDRESS:-auto}"

[network]
max_peers = 128
min_peers = 8
peer_discovery_interval_seconds = 300

[consensus]
type = "aBFT"
finality_time_seconds = 3

[storage]
data_dir = "./node_data/validator-1"
prune_enabled = true

[logging]
level = "INFO"
format = "json"
```

> **Note:** Environment variable substitution (e.g. `${LOS_VALIDATOR_ADDRESS}`) is supported. See the full `validator.toml` in the repository root for all available options including sentry node architecture configuration.

---

## Validator Rewards

### How Rewards Work

Rewards come from a fixed pool of 500,000 LOS (non-inflationary — already part of the total supply). They are distributed each epoch to eligible validators.

| Parameter | Value |
|---|---|
| **Total Pool** | 500,000 LOS |
| **Per Epoch** | 5,000 LOS (halves every 48 epochs) |
| **Formula** | `reward_i = budget × stake_i / Σ(all_stakes)` |
| **Math** | Pure linear integer arithmetic — no floating-point, Sybil-neutral |
| **Min Stake (Rewards)** | 1,000 LOS |
| **Min Uptime** | ≥95% |

### Halving Schedule

| Epoch Range | Reward Per Epoch |
|---|---|
| 0–47 | 5,000 LOS |
| 48–95 | 2,500 LOS |
| 96–143 | 1,250 LOS |
| 144–191 | 625 LOS |
| ... | Continues halving |

### Check Reward Status

```bash
curl http://localhost:3030/reward-info | python3 -m json.tool
```

---

## Slashing & Penalties

Validators are held accountable for misbehavior:

| Offense | Detection | Penalty |
|---|---|---|
| **Double-signing** | Conflicting block signatures | 100% stake slashed, permanent ban |
| **Fraudulent transaction** | Multi-validator verification | 100% stake slashed, permanent ban |
| **Extended downtime** | Uptime tracking (<95% over observation window) | 1% of stake slashed |

### Check Slashing Status

```bash
# Global slashing stats
curl http://localhost:3030/slashing | python3 -m json.tool

# Your validator's slashing profile
curl http://localhost:3030/slashing/YOUR_ADDRESS | python3 -m json.tool
```

---

## Monitoring

### Health Check

```bash
curl http://localhost:3030/health
```

Response:
```json
{
  "status": "healthy",
  "version": "2.2.0",
  "uptime_seconds": 86400,
  "chain": { "accounts": 8, "blocks": 42, "id": "los-mainnet" },
  "database": { "accounts_count": 8, "blocks_count": 42, "size_on_disk": 524287 }
}
```

### Prometheus Metrics

```bash
curl http://localhost:3030/metrics
```

Key metrics:
- `los_active_validators` — Number of active validators
- `los_blocks_total` — Total blocks processed
- `los_accounts_total` — Total accounts
- `los_consensus_rounds` — aBFT rounds completed
- `los_peer_count` — Connected peers
- `los_uptime_seconds` — Node uptime

### Peer Connectivity

```bash
curl http://localhost:3030/peers | python3 -m json.tool
```

### Consensus Status

```bash
curl http://localhost:3030/consensus | python3 -m json.tool
```

---

## Maintenance & Upgrades

### Upgrading the Node

```bash
cd /opt/unauthority-core
git pull
source ~/.cargo/env
cargo build --release -p los-node --features mainnet
sudo systemctl stop los-node
sudo cp target/release/los-node /usr/local/bin/los-node
sudo systemctl start los-node
```

### Backup

The data directory contains:
```
/opt/los-node/
├── los_database/          # RocksDB ledger data
├── checkpoints/           # Periodic state checkpoints
├── wallet.json.enc        # Encrypted wallet (KEEP THIS SAFE)
└── pid.txt                # Process ID (auto-generated)
```

**Critical:** Back up `wallet.json.enc` — it contains your Dilithium5 keypair. If lost, your validator identity and staked tokens are unrecoverable.

### Unregistering

```bash
curl -X POST http://localhost:3030/unregister-validator \
  -H "Content-Type: application/json" \
  -d '{
    "address": "YOUR_LOS_ADDRESS",
    "public_key": "HEX_PUBLIC_KEY",
    "signature": "HEX_SIGNATURE"
  }'
```

---

## Troubleshooting

### Node won't start

| Symptom | Cause | Fix |
|---|---|---|
| `Error: wallet password required` | Missing `LOS_WALLET_PASSWORD` | Set the env variable |
| `Error: cannot bind to port` | Port already in use | Change `--port` or kill conflicting process |
| `Error: database locked` | Another instance running | Stop the other instance first |

### No peers connecting

| Check | Command | Expected |
|---|---|---|
| Tor running | `systemctl status tor` | Active (running) |
| SOCKS5 reachable | `curl --socks5-hostname 127.0.0.1:9050 http://check.torproject.org` | HTML response |
| Health endpoint | `curl http://localhost:3030/health` | JSON with `"status":"healthy"` |
| Peer list | `curl http://localhost:3030/peers` | `peer_count > 0` |

### Tor hidden service not reachable

1. Check `/var/lib/tor/los-validator/` permissions — must be owned by the Tor user
2. Verify port mappings in `torrc` match your `--port`
3. Wait 30–60 seconds after Tor restart for the hidden service to propagate
4. Test from another machine: `curl --socks5-hostname 127.0.0.1:9050 http://YOUR_ONION.onion:3030/health`

### Node syncing slowly

- Tor adds latency (2–5 seconds per hop). This is by design for privacy.
- Ensure your Tor relay has good bandwidth settings
- Check peer count — more peers = faster sync
