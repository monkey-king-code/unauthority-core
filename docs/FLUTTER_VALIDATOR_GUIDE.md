# How to Run a Validator Node — Flutter Validator Dashboard

A step-by-step guide to running an Unauthority (LOS) validator node using the **Flutter Validator Dashboard** app. No command-line experience required.

> **Looking for the CLI guide?** See [VALIDATOR_GUIDE.md](VALIDATOR_GUIDE.md) for terminal-based setup.

---

## Table of Contents

1. [Overview](#overview)
2. [Requirements](#requirements)
3. [Download & Install](#download--install)
4. [First Launch — Tor Setup](#first-launch--tor-setup)
5. [Wallet Setup — Generate or Import Keys](#wallet-setup--generate-or-import-keys)
6. [Starting the Node](#starting-the-node)
7. [Validator Registration](#validator-registration)
8. [Dashboard Monitoring](#dashboard-monitoring)
9. [Connecting to External Peers](#connecting-to-external-peers)
10. [Backup & Recovery](#backup--recovery)
11. [Updating the App](#updating-the-app)
12. [Troubleshooting](#troubleshooting)

---

## Overview

The **LOS Validator Node** app is a desktop application that bundles everything you need to run a full validator:

- **`los-node` binary** — the full validator node (bundled, no separate install)
- **Tor Expert Bundle** — auto-downloaded on first launch (no Tor Browser needed)
- **CRYSTALS-Dilithium5** — post-quantum cryptography via native Rust FFI
- **Dashboard** — real-time monitoring, key management, and consensus tracking

Your node runs as a **Tor Hidden Service** (`.onion` address) — fully private, no port forwarding or domain needed.

---

## Requirements

| Component | Minimum |
|---|---|
| **OS** | macOS 12+, Windows 10+, or Linux (Ubuntu 22.04+) |
| **CPU** | 2 cores |
| **RAM** | 4 GB |
| **Disk** | 10 GB SSD |
| **Internet** | Any stable connection |
| **Stake** | 1 LOS to register; 1,000 LOS for reward eligibility |

> **No Rust/Flutter/CLI knowledge needed.** The app handles everything automatically.

---

## Download & Install

### Step 1: Download

Go to **[GitHub Releases](https://github.com/monkey-king-code/unauthority-core/releases)** and download the latest version for your platform:

| Platform | File |
|---|---|
| macOS | `LOS-Validator-*-macos.dmg` |
| Windows | `LOS-Validator-*-windows-x64.zip` |
| Linux | `LOS-Validator-*-linux-x64.tar.gz` |

### Step 2: Install

**macOS:**
1. Open the `.dmg` file
2. Drag **LOS Validator & Miner** to `/Applications`
3. **Important:** The binary is unsigned. Remove quarantine:
   ```
   xattr -cr /Applications/LOS\ Validator\ \&\ Miner.app
   ```
   Or: **System Settings → Privacy & Security → Open Anyway**

**Windows:**
1. Extract the `.zip` to any folder
2. Right-click both `flutter_validator.exe` AND `los-node.exe` → **Properties** → check **Unblock** → OK
3. If Windows SmartScreen appears: click **More info** → **Run anyway**

**Linux:**
1. Extract the `.tar.gz`:
   ```
   tar xzf LOS-Validator-*.tar.gz
   ```
2. Make files executable:
   ```
   chmod +x run.sh flutter_validator los-node
   ```
3. Launch with:
   ```
   ./run.sh
   ```
   (This sets `LD_LIBRARY_PATH` for the native crypto library)

---

## First Launch — Tor Setup

When you open the app for the first time:

1. The app **automatically downloads the Tor Expert Bundle** (~20 MB)
2. Wait 1-2 minutes for the download and extraction to complete
3. The app starts a Tor process and generates a unique `.onion` address for your node
4. This `.onion` address is your node's identity on the network — it's created automatically

> **No manual Tor configuration needed.** The app handles Tor setup, hidden service generation, and peer discovery entirely on its own.

You can see the Tor connection status in the dashboard. A green indicator means Tor is connected and your hidden service is active.

---

## Wallet Setup — Generate or Import Keys

Before running a validator, you need a wallet (keypair) to sign blocks and receive rewards.

### Option A: Generate New Keys

1. On first launch, the app shows the **Wallet Setup** screen
2. Click **"Generate New Wallet"**
3. A **24-word BIP39 seed phrase** is displayed

> **⚠️ CRITICAL: Write down your seed phrase on paper. Store it in a safe, offline location.**
>
> - Your seed phrase is the ONLY way to recover your wallet
> - If you lose it, your LOS and validator stake are **permanently lost**
> - **Never** store it digitally (no screenshots, no notes app, no cloud)
> - **Never** share it with anyone

4. Confirm you've saved the seed phrase
5. Your wallet address is generated using **CRYSTALS-Dilithium5** (post-quantum secure)

### Option B: Import Existing Keys

1. Click **"Import Existing Wallet"**
2. Enter your **24-word seed phrase** (space-separated)
3. The app derives the same keypair deterministically
4. Your existing wallet address and balance appear in the dashboard

> **Format:** 24 English words separated by spaces. Example: `abandon ability able about above absent absorb abstract absurd abuse access accident ...`

---

## Starting the Node

### Step 1: Start the Bundled Node

1. In the main dashboard, press **"START NODE"**
2. The bundled `los-node` binary launches automatically
3. The node:
   - Creates a Tor hidden service (`.onion` address)
   - Connects to seed peers via the Tor network
   - Begins syncing the blockchain from genesis
   - Announces itself to the peer network

### Step 2: Wait for Sync

- **First sync** may take several minutes depending on chain height
- The dashboard shows:
  - **Block Height** — current synced block
  - **Peer Count** — number of connected validators
  - **Sync Status** — "Syncing..." or "Synced"
- Wait until status shows **"Synced"** before registering as a validator

### Step 3: Verify Connectivity

- **Peers:** You should see 3+ connected peers within a few minutes
- **Tor:** Green status indicator = connected
- **Block Height:** Should match the network's current height

> **Tip:** The node continues running in the background while the dashboard is open. Closing the app stops the node. For 24/7 operation, keep the app running or consider the [CLI setup](VALIDATOR_GUIDE.md#systemd-service-production) with systemd.

---

## Validator Registration

To participate in consensus and earn rewards, you must register as a validator.

### Prerequisites

- Node is **fully synced** (block height matches network)
- Your wallet holds **≥ 1 LOS** (minimum registration stake; 1,000 LOS for rewards)
- Node has been running for at least a few minutes (peers discovered)

### How to Register

1. Navigate to the **Validator** section in the dashboard
2. Click **"Register as Validator"**
3. Enter your **stake amount** (minimum 1 LOS to register; 1,000 LOS for reward eligibility)
4. Confirm the registration transaction
5. Wait for the transaction to be finalized (typically < 3 seconds)

### After Registration

- Your node is now a **registered validator**
- It participates in **aBFT consensus** (block production and voting)
- Rewards are earned proportionally to stake (linear, Sybil-neutral) — see [Reward Rules](#reward-rules) below
- **Maintain ≥ 95% uptime** to remain eligible for rewards

### Reward Rules

| Parameter | Value |
|---|---|
| **Reward Pool** | 500,000 LOS (non-inflationary, fixed) |
| **Rate** | 5,000 LOS/epoch, halving every 48 epochs |
| **Formula** | `reward_i = budget × stake_i / Σ(stake_all)` (linear) |
| **Min Stake** | 1 LOS (register) / 1,000 LOS (rewards) |
| **Min Uptime** | 95% |

> **Linear Voting:** LOS uses linear voting (1 LOS = 1 vote). This is Sybil-neutral — splitting stake across multiple identities yields the same total power.

---

## Dashboard Monitoring

The dashboard provides real-time information about your validator:

### Main Dashboard

| Panel | Description |
|---|---|
| **Node Status** | Running / Stopped / Error |
| **Block Height** | Current chain height |
| **Peer Count** | Connected peer nodes |
| **Tor Status** | Hidden service connectivity |
| **Your Address** | Your validator's public address |
| **Balance** | Current LOS balance |

### Validator Metrics

| Metric | Description |
|---|---|
| **Uptime** | Your node's uptime percentage |
| **Finality Time** | Average block finality (target: < 3s) |
| **Blocks Produced** | Blocks your node has authored |
| **Rewards Earned** | Total LOS earned from validation |
| **Slashing Events** | Any penalty events (should be 0) |
| **Consensus Participation** | Your voting activity in aBFT rounds |

### Logs

The dashboard shows live node logs. Key entries to look for:

| Log | Meaning |
|---|---|
| `Tor hidden service ready` | Your .onion address is active |
| `Connected to X peers` | Successfully discovered peers |
| `Block #N finalized` | Consensus is working |
| `Validator registered` | Registration confirmed |
| `Reward distributed: X CIL` | You received validation rewards |

---

## Connecting to External Peers

The dashboard always connects to **external peers** for API data — never its own local node. This prevents a compromised local node from displaying false consensus information.

### Automatic (Default)

The app automatically:
1. Downloads the seed peer list
2. Pings available peers to check latency
3. Connects to the most stable external peer

### Manual Peer Override

1. Go to **Settings**
2. Enter a specific node endpoint: `http://<onion-address>.onion:3030`
3. Click **Test Connection** to verify
4. Click **Save**

> **Note:** All connections go through Tor — no direct IP exposure.

---

## Backup & Recovery

### What to Back Up

| Data | Location | How |
|---|---|---|
| **Seed Phrase** | Your paper backup | Write it down during wallet setup |
| **App Data** | Platform-specific (see below) | Copy the entire folder |

### App Data Locations

| Platform | Path |
|---|---|
| macOS | `~/Library/Application Support/com.unauthority.flutter_validator/` |
| Windows | `%APPDATA%\com.unauthority.flutter_validator\` |
| Linux | `~/.local/share/com.unauthority.flutter_validator/` |

### Recovery on a New Machine

1. Install the LOS Validator Node app on the new machine
2. Launch the app → **Import Existing Wallet**
3. Enter your 24-word seed phrase
4. Your wallet, address, and balance are restored
5. Start the node — it will sync from the network

> **Your seed phrase is everything.** As long as you have it, you can recover your wallet on any machine. The blockchain stores your balance and validator registration — they sync automatically.

---

## Updating the App

When a new version is released:

1. Download the latest release from [GitHub Releases](https://github.com/monkey-king-code/unauthority-core/releases)
2. **Stop the node** in the dashboard first
3. Replace the old app with the new version:
   - **macOS:** Drag new `.app` to `/Applications` (replace)
   - **Windows:** Extract new `.zip` over the old folder
   - **Linux:** Extract new `.tar.gz` over the old folder
4. Launch the updated app
5. Your wallet data is preserved (stored separately from the app)
6. Start the node — it resumes from where it left off

> **Always update promptly.** Outdated nodes may miss consensus changes and could be slashed for protocol violations.

---

## Troubleshooting

### Common Issues

**"Tor connection failed"**
- Check your internet connection
- The app may need to re-download the Tor Expert Bundle
- On corporate networks, Tor may be blocked — try a different network
- macOS: Ensure the app has network permission in System Settings → Privacy

**"No peers found"**
- Wait 2-3 minutes — peer discovery over Tor takes time
- Check that Tor status shows green (connected)
- Verify the seed peer list is reachable

**"Node won't start"**
- macOS: Run `xattr -cr /Applications/LOS\ Validator\ \&\ Miner.app` to remove quarantine
- Windows: Ensure both `.exe` files are unblocked (Properties → Unblock)
- Linux: Ensure `los-node` has execute permission (`chmod +x los-node`)
- Check that no other instance is already running

**"Sync stuck"**
- Restart the node (Stop → Start)
- Check peer count — if 0, there may be a Tor connectivity issue
- Check logs for error messages

**"Registration failed"**
- Ensure your balance is ≥ 1 LOS (registration) or ≥ 1,000 LOS (reward eligibility)
- Ensure the node is fully synced
- Wait a few seconds and retry

**"Balance shows 0"**
- Wait for sync to complete — balance appears after full sync
- Verify you're connected to mainnet peers (not testnet)
- Try restarting the app

### macOS-Specific

- **"App is damaged"**: Run `xattr -cr` as shown in the install section
- **Gatekeeper block**: System Settings → Privacy & Security → Open Anyway
- **Native library error**: The `liblos_crypto_ffi.dylib` must be in the app's `Frameworks` directory

### Windows-Specific

- **SmartScreen warning**: Click "More info" → "Run anyway"
- **DLL not found**: Ensure `los_crypto_ffi.dll` is next to the `.exe`
- **Firewall**: Windows Firewall may block Tor — allow the app through

### Linux-Specific

- **Library not found**: Run via `./run.sh` (sets `LD_LIBRARY_PATH`)
- **Permissions**: `chmod +x flutter_validator los-node run.sh`
- **Display issues**: Ensure you have a compatible desktop environment (GNOME, KDE, etc.)

---

## PoW Mining (Public Distribution)

Validators can earn additional LOS by participating in **Proof-of-Work mining**. Mining distributes the public supply of 21,158,413 LOS over time.

### How It Works

- Mining runs as a **background thread** inside your validator node
- The algorithm is **SHA3-256**: `SHA3(LOS_MINE_V1 ‖ chain_id ‖ address ‖ epoch ‖ nonce)`
- Each epoch lasts **1 hour** (mainnet) or **2 minutes** (testnet)
- Reward: **100 LOS/epoch**, halving every 8,760 epochs (~1 year)
- **One reward per address per epoch** — no double-mining

### Enabling Mining in the Dashboard

1. Go to **Settings** → **Mining**
2. Toggle **"Enable Mining"** to ON
3. Set **Mining Threads** (default: 1 thread; more threads = higher chance per epoch)
4. **Restart the node** for changes to take effect

> **Requirement:** You must be running a full validator node. Mining is built into the node binary — there is no external mining API.

### Monitoring Mining

| Dashboard Field | Meaning |
|---|---|
| **Mining Status** | Active / Inactive |
| **Current Epoch** | The epoch your miner is working on |
| **Hashrate** | Approximate hashes/second |
| **Blocks Mined** | Total successful mining proofs |
| **Mining Rewards** | Total LOS earned from PoW mining |

### Mining vs Validator Rewards

| | Validator Rewards | Mining Rewards |
|---|---|---|
| **Source** | 500,000 LOS pool | 21,158,413 LOS pool |
| **Requirement** | ≥ 1,000 LOS stake + 95% uptime | Run node with mining enabled |
| **Distribution** | Proportional to stake (linear) | First valid proof per epoch wins |
| **Halving** | Every 48 epochs (~4 years) | Every 8,760 epochs (~1 year) |

---

## FAQ

**Q: Do I need to install Tor separately?**
A: No. The app downloads and manages Tor automatically.

**Q: Can I run the validator on a VPS?**
A: The Flutter app requires a desktop environment. For headless VPS servers, use the [CLI guide](VALIDATOR_GUIDE.md) with `los-node` directly.

**Q: How much LOS do I need to start?**
A: 1 LOS minimum to register as a validator. 1,000 LOS minimum for reward eligibility.

**Q: Is my IP address exposed?**
A: No. All network traffic goes through Tor. Your node is only reachable via its `.onion` address.

**Q: Can I run multiple validators?**
A: Each validator needs a unique seed phrase and separate app instance. Running multiple on the same machine is not recommended.

**Q: What happens if my computer goes offline?**
A: Your validator temporarily stops participating in consensus. If downtime exceeds 5%, you lose reward eligibility. Extended downtime may result in minor slashing. When you come back online, the node re-syncs and resumes automatically.

**Q: Are my keys post-quantum secure?**
A: Yes. Unauthority uses **CRYSTALS-Dilithium5**, a NIST-standardized post-quantum signature scheme. Your keys are resistant to both classical and quantum computer attacks.

---

## Related Documentation

- [Validator Guide (CLI)](VALIDATOR_GUIDE.md) — Terminal-based setup for servers/VPS
- [Tor Setup](TOR_SETUP.md) — Tor hidden service details
- [API Reference](API_REFERENCE.md) — REST API endpoints
- [Architecture](ARCHITECTURE.md) — System design overview
- [Whitepaper](WHITEPAPER.md) — Technical specification

---

*Unauthority (LOS) — 100% Immutable, Permissionless, Decentralized.*
