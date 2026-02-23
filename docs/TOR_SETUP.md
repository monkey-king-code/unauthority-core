# Tor Setup Guide — Unauthority (LOS) v2.0.0

Complete guide to configuring Tor hidden services for Unauthority validators and clients.

---

## Table of Contents

1. [Why Tor?](#why-tor)
2. [Install Tor](#install-tor)
3. [Hidden Service Configuration](#hidden-service-configuration)
4. [Single Validator Setup](#single-validator-setup)
5. [Multi-Validator Setup (Same Host)](#multi-validator-setup-same-host)
6. [Verify Hidden Service](#verify-hidden-service)
7. [Auto-Bootstrap (v1.0.9+)](#auto-bootstrap-v109)
8. [Automatic Hidden Service Generation (v1.0.9+)](#automatic-hidden-service-generation-v109)
9. [Flutter App Tor Connectivity](#flutter-app-tor-connectivity)
10. [Firewall Configuration](#firewall-configuration)
11. [Performance Tuning](#performance-tuning)
12. [Troubleshooting](#troubleshooting)

---

## Why Tor?

Unauthority runs **exclusively** on Tor hidden services:

- **No IP exposure** — validators cannot be DDoS'd by IP
- **No DNS** — `.onion` addresses are cryptographically derived
- **No clearnet dependency** — fully permissionless
- **NAT traversal** — works behind any firewall/router
- **Censorship resistant** — cannot be blocked by IP/domain

Both Mainnet and Testnet run on the live Tor network. There is no `localhost`-only testing mode.

---

## Install Tor

### Linux (Ubuntu/Debian)

```bash
sudo apt update
sudo apt install tor -y
sudo systemctl enable tor
sudo systemctl start tor
```

### macOS

```bash
brew install tor
brew services start tor
```

### Verify Installation

```bash
# Check Tor is running
systemctl status tor        # Linux
brew services info tor      # macOS

# Verify SOCKS5 proxy is listening
ss -tlnp | grep 9050       # Linux
lsof -i :9050              # macOS

# Test connectivity
curl --socks5-hostname 127.0.0.1:9050 https://check.torproject.org/api/ip
```

Expected output: `{"IsTor":true,"IP":"..."}` (an exit node IP).

---

## Hidden Service Configuration

### How It Works

Tor hidden services work by:
1. Tor generates a unique `.onion` address (public key derived)
2. Tor opens a local socket listening on specified ports
3. External users connect via `.onion` → Tor routes to your local port
4. Your real IP is never exposed

### Port Architecture

Each validator needs **two** Tor-routed ports:

| Service | Local Port | Virtual Port (.onion) | Protocol |
|---|---|---|---|
| REST API | 3030 | 3030 | HTTP/JSON |
| P2P Gossip | 4030 | 4030 | HTTP/JSON |

> **Note:** gRPC (port 23030) binds locally but is NOT routed through Tor by default. Add it if you need remote gRPC access.

### Port Derivation Scheme

For multi-validator setups, port numbers are derived from the `--port` flag:

| Flag | REST Port | P2P Port | gRPC Port |
|---|---|---|---|
| `--port 3030` | 3030 | 4030 | 23030 |
| `--port 3031` | 3031 | 4031 | 23031 |
| `--port 3032` | 3032 | 4032 | 23032 |
| `--port 3033` | 3033 | 4033 | 23033 |

Formula: `P2P = REST + 1000`, `gRPC = REST + 20000`

---

## Single Validator Setup

### 1. Generate Hidden Service

Edit the Tor configuration:

```bash
sudo nano /etc/tor/torrc
```

Add at the bottom:

```
# --- LOS Validator ---
HiddenServiceDir /var/lib/tor/los_validator/
HiddenServicePort 3030 127.0.0.1:3030
HiddenServicePort 4030 127.0.0.1:4030
```

### 2. Restart Tor

```bash
sudo systemctl restart tor
```

### 3. Get Your .onion Address

```bash
sudo cat /var/lib/tor/los_validator/hostname
```

Output: `abcdef1234567890abcdef1234567890abcdef1234567890abcdefgh.onion`

### 4. Start the Validator

```bash
los-node --port 3030 --data-dir /opt/los-nodes/v1 --node-id my-validator
```

The node will auto-detect Tor SOCKS5 at `127.0.0.1:9050` and auto-bootstrap peers from genesis.

---

## Multi-Validator Setup (Same Host)

Running 4 validators on one machine (e.g., for testnet bootstrap):

### Tor Configuration

```bash
sudo nano /etc/tor/torrc
```

```
# --- Validator 1 ---
HiddenServiceDir /var/lib/tor/los_v1/
HiddenServicePort 3030 127.0.0.1:3030
HiddenServicePort 4030 127.0.0.1:4030

# --- Validator 2 ---
HiddenServiceDir /var/lib/tor/los_v2/
HiddenServicePort 3031 127.0.0.1:3031
HiddenServicePort 4031 127.0.0.1:4031

# --- Validator 3 ---
HiddenServiceDir /var/lib/tor/los_v3/
HiddenServicePort 3032 127.0.0.1:3032
HiddenServicePort 4032 127.0.0.1:4032

# --- Validator 4 ---
HiddenServiceDir /var/lib/tor/los_v4/
HiddenServicePort 3033 127.0.0.1:3033
HiddenServicePort 4033 127.0.0.1:4033
```

```bash
sudo systemctl restart tor
```

### Get All .onion Addresses

```bash
for i in v1 v2 v3 v4; do
  echo "$i: $(sudo cat /var/lib/tor/los_$i/hostname)"
done
```

### Start All Validators

```bash
los-node --port 3030 --data-dir /opt/los-nodes/v1 --node-id validator-1 &
los-node --port 3031 --data-dir /opt/los-nodes/v2 --node-id validator-2 &
los-node --port 3032 --data-dir /opt/los-nodes/v3 --node-id validator-3 &
los-node --port 3033 --data-dir /opt/los-nodes/v4 --node-id validator-4 &
```

### Systemd Services (Recommended)

For production, use systemd. See [Validator Guide — Systemd](VALIDATOR_GUIDE.md#systemd-service-recommended).

---

## Verify Hidden Service

### Test from Another Machine

```bash
# Install torsocks
sudo apt install torsocks    # Linux
brew install torsocks         # macOS

# Test REST API
torsocks curl http://YOUR_ONION.onion:3030/health

# Test P2P port
torsocks curl http://YOUR_ONION.onion:4030/
```

### Test Locally

```bash
# Via SOCKS5 proxy
curl --socks5-hostname 127.0.0.1:9050 http://YOUR_ONION.onion:3030/health

# Direct localhost (bypasses Tor)
curl http://127.0.0.1:3030/health
```

### Expected Response

```json
{
  "status": "ok",
  "version": "2.0.0",
  "network": "mainnet",
  "accounts": 8,
  "blocks": 0,
  "supply": "21936236",
  "peer_count": 4
}
```

---

## Auto-Bootstrap (v1.0.9+)

As of v1.0.9, `los-node` automatically bootstraps peer connections:

### How It Works

1. Node reads `genesis_config.json` (embedded in binary at compile-time)
2. Extracts `.onion` addresses and ports from bootstrap validator entries
3. Auto-detects Tor SOCKS5 proxy at `127.0.0.1:9050` (500ms timeout)
4. Connects to all bootstrap peers via Tor
5. Downloads updated peer table from connected peers

### Manual Override

You can override auto-bootstrap with environment variables:

```bash
# Custom peer list (overrides genesis bootstrap)
export LOS_BOOTSTRAP_NODES="abc123.onion:4030,def456.onion:4031"

# Custom Tor SOCKS5 address
export LOS_SOCKS5_PROXY="socks5h://127.0.0.1:9150"  # e.g., Tor Browser port

# Custom genesis file path
export LOS_GENESIS_PATH="/path/to/custom/genesis_config.json"
```

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `LOS_BOOTSTRAP_NODES` | *(from genesis)* | Comma-separated `onion:port` list |
| `LOS_SOCKS5_PROXY` | `127.0.0.1:9050` | Tor SOCKS5 proxy address |
| `LOS_GENESIS_PATH` | *(embedded)* | Path to genesis config |
| `LOS_NETWORK` | `mainnet` | Network: `mainnet` or `testnet` |
| `LOS_DATA_DIR` | `./data` | Data directory |
| `LOS_LOG_LEVEL` | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| `LOS_REST_PORT` | `3030` | REST API port (alternative to `--port`) |
| `LOS_NODE_ID` | `node` | Node identifier |

---

## Automatic Hidden Service Generation (v1.0.9+)

As of v1.0.9, `los-node` can **automatically generate** a unique Tor Hidden Service (`.onion` address) on startup — no manual `torrc` editing required.

### Prerequisites

Tor must be running with the **Control Port** enabled:

```bash
# Add to /etc/tor/torrc (Linux) or /usr/local/etc/tor/torrc (macOS Homebrew)
ControlPort 9051
CookieAuthentication 1
```

Then restart Tor:

```bash
sudo systemctl restart tor    # Linux
brew services restart tor     # macOS
```

### How It Works

1. Node checks if `LOS_ONION_ADDRESS` is already set (manual config)
2. If not set, probes the Tor Control Port (default `127.0.0.1:9051`)
3. Authenticates via **Cookie** (preferred), **Password**, or **Null** auth
4. Sends `ADD_ONION` command to create an **ephemeral ED25519-V3** hidden service
5. Maps the node's REST, P2P, and gRPC ports to the `.onion` address
6. Persists the generated private key to `{data_dir}/tor_hidden_service_key`
7. Sets `LOS_ONION_ADDRESS` for the running process
8. Registers the `.onion` address with the network for peer discovery

On subsequent startups, if the key file exists, the same `.onion` address is reused.

### Graceful Fallback

Auto-generation is **non-fatal**. If the Control Port is unavailable or authentication fails, the node logs a warning and continues normally. This means:

- **Existing manual setups** (`torrc` + `LOS_ONION_ADDRESS`) continue to work unchanged
- **Nodes without Tor Control Port** still function (but without auto `.onion`)
- **No breaking changes** to current deployments

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `LOS_TOR_CONTROL` | `127.0.0.1:9051` | Tor Control Port address |
| `LOS_TOR_COOKIE_PATH` | *(auto-detected)* | Path to Tor control auth cookie |
| `LOS_TOR_CONTROL_PWD` | *(none)* | Tor control password (if using `HashedControlPassword`) |
| `LOS_ONION_ADDRESS` | *(none)* | Skip auto-gen; use this `.onion` address directly |

### Cookie Path Auto-Detection

The node searches these paths in order:

| OS | Path |
|---|---|
| macOS (Homebrew) | `/usr/local/var/lib/tor/control_auth_cookie` |
| macOS (Homebrew) | `/opt/homebrew/var/lib/tor/control_auth_cookie` |
| Linux (Debian/Ubuntu) | `/var/run/tor/control.authcookie` |
| Linux (systemd) | `/run/tor/control.authcookie` |
| Linux (var/lib) | `/var/lib/tor/control_auth_cookie` |

If none are found, set `LOS_TOR_COOKIE_PATH` explicitly or use password authentication.

### Key Persistence

The generated ED25519-V3 private key is saved to:

```
{data_dir}/tor_hidden_service_key
```

This file has `0600` permissions (owner-only read/write). **Back up this file** — if lost, a new `.onion` address will be generated and you must re-register with the network.

### Example: Zero-Config Validator

With Tor Control Port enabled, starting a validator requires no `.onion` configuration:

```bash
# Tor handles everything automatically
./los-node --port 3030 --p2p-port 4030 --node-id v1 --data-dir ./data/v1

# Output:
# [INFO] Tor Control Port available at 127.0.0.1:9051
# [INFO] Authenticated with Tor via cookie
# [INFO] Generated Tor Hidden Service: abc123...xyz.onion
# [INFO] Port mappings: 3030→3030, 4030→4030, 23030→23030
# [INFO] Onion address registered for peer discovery
```

### Example: Password Authentication

If cookie auth is not available:

```bash
# Generate hashed password for torrc
tor --hash-password "my_secure_password"
# Copy output to torrc: HashedControlPassword 16:...

# Set password for the node
export LOS_TOR_CONTROL_PWD="my_secure_password"
./los-node --port 3030 --p2p-port 4030 --node-id v1
```

---

## Flutter App Tor Connectivity

Flutter wallet and validator apps include a **bundled Tor client** — no system Tor required.

### Connection Flow

1. App starts bundled Tor binary (platform-specific)
2. Tor establishes SOCKS5 proxy on a random local port
3. App fetches peer list from bootstrap `.onion` nodes
4. Latency check: pings all discovered peers
5. Selects most stable/fastest peer as primary connection
6. All API calls routed through Tor SOCKS5 to `.onion` peer

### Network Config (Flutter)

Located at `flutter_wallet/assets/network_config.json` and `flutter_validator/assets/network_config.json`:

```json
{
  "mainnet": {
    "bootstrap_nodes": [
      {
        "onion": "f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion",
        "rest_port": 3030,
        "p2p_port": 4030
      }
    ]
  }
}
```

### Validator Dashboard Constraint

The `flutter_validator` app **never** connects to its own local node for API data. It always connects to external peers to verify network consensus integrity. This prevents a compromised local node from displaying false data.

---

## Firewall Configuration

Since Tor handles NAT traversal, firewall rules are minimal:

### Required (Outbound)

| Port | Direction | Purpose |
|---|---|---|
| 9050 | Loopback | Tor SOCKS5 proxy |
| 9001 | Outbound | Tor relay connections (OR port) |
| 443/80 | Outbound | Tor directory authorities |

### Optional (Local Access)

| Port | Direction | Purpose |
|---|---|---|
| 3030-3033 | Loopback | REST API (local only) |
| 4030-4033 | Loopback | P2P gossip (local only) |
| 23030-23033 | Loopback | gRPC (local only) |

### Production Lockdown

```bash
# Allow loopback
sudo ufw allow in on lo

# Allow SSH (for server management)
sudo ufw allow 22/tcp

# Allow Tor outbound (already allowed by default)

# Block everything else inbound
sudo ufw default deny incoming
sudo ufw default allow outgoing

sudo ufw enable
```

> **Important:** Do NOT open ports 3030-4033 to the public internet. All external access goes through Tor.

---

## Performance Tuning

### Tor Circuit Optimization

Add to `/etc/tor/torrc`:

```
# Reduce circuit build timeout for faster connections
CircuitBuildTimeout 30
LearnCircuitBuildTimeout 0

# Allow more simultaneous circuits
MaxCircuitDirtiness 600

# Increase SOCKS timeout for slow networks
SocksTimeout 120
```

### System Limits (Linux)

```bash
# Increase file descriptors
echo "* soft nofile 65535" >> /etc/security/limits.conf
echo "* hard nofile 65535" >> /etc/security/limits.conf

# Increase for systemd services
# Add to [Service] section:
LimitNOFILE=65535
```

### Expected Performance

| Metric | Local | Over Tor |
|---|---|---|
| API response | <5ms | 500ms - 2s |
| P2P gossip round-trip | <1ms | 1s - 3s |
| Transaction finality | <100ms | <3s |
| Block propagation | <10ms | 1s - 2s |

---

## Troubleshooting

### Tor Not Running

```bash
# Check status
systemctl status tor          # Linux
brew services info tor        # macOS

# Check logs
journalctl -u tor -f          # Linux
cat /usr/local/var/log/tor/log    # macOS (Homebrew)

# Restart
sudo systemctl restart tor    # Linux
brew services restart tor     # macOS
```

### Hidden Service Not Created

```bash
# Check directory permissions
ls -la /var/lib/tor/

# Tor must own the directory
sudo chown -R debian-tor:debian-tor /var/lib/tor/los_validator/
sudo chmod 700 /var/lib/tor/los_validator/

# Verify torrc syntax
tor --verify-config
```

### No Peers Connecting

```bash
# Check if node can reach Tor
curl --socks5-hostname 127.0.0.1:9050 http://check.torproject.org/api/ip

# Check if bootstrap peers are reachable
curl --socks5-hostname 127.0.0.1:9050 \
  http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030/health

# Check node peer count
curl http://127.0.0.1:3030/peers

# Check node logs for connection errors
journalctl -u los-node-v1 -f | grep -i "peer\|tor\|connect"
```

### SOCKS5 Connection Refused

```bash
# Default Tor port: 9050
# Tor Browser port: 9150

# Check which port Tor is using
ss -tlnp | grep tor    # Linux
lsof -i -P | grep tor  # macOS

# Override if non-standard
export LOS_SOCKS5_PROXY="socks5h://127.0.0.1:9150"
```

### Slow Connections

- Tor adds 500ms-2s latency per hop — this is expected
- First connection after startup is slowest (circuit building)
- Subsequent requests reuse circuits and are faster
- If consistently >5s, check `CircuitBuildTimeout` in torrc
- Consider running a Tor relay (not exit) for better circuit priority

### .onion Address Changed After Restart

The `.onion` address is derived from keys in `HiddenServiceDir`. If the directory was deleted or permissions changed, Tor generates new keys:

```bash
# Backup your hidden service keys
sudo cp -r /var/lib/tor/los_validator/ /backup/tor_keys/

# After restore, fix permissions
sudo chown -R debian-tor:debian-tor /var/lib/tor/los_validator/
sudo chmod 700 /var/lib/tor/los_validator/
sudo systemctl restart tor
```

> **Critical:** If your `.onion` address changes, you must re-register with the network. Other nodes will not recognize the new address until updated.
