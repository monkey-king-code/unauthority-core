# LOS Validator Node

Validator node dashboard for **Unauthority (LOS)** blockchain. Track node status, manage keys, and monitor consensus participation.

[![Version](https://img.shields.io/badge/version-2.0.2-blue)]()
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](../LICENSE)

---

## Features

- **Live Dashboard** — real-time validator stats, uptime, and peer connections
- **Key Management** — generate or import validator keys with BIP39 seed phrases
- **Node Monitoring** — block height, finality times, transaction throughput
- **Slashing Alerts** — track penalties and validator health
- **Consensus Status** — aBFT safety parameters and quorum tracking
- **Bundled los-node** — includes full validator binary (no separate install needed)
- **Built-in Tor** — auto-downloads Tor Expert Bundle (no Tor Browser needed)
- **CRYSTALS-Dilithium5** — post-quantum digital signatures via native Rust FFI

---

## Download

Pre-built releases for macOS, Windows, and Linux:

**[Download from GitHub Releases](https://github.com/monkey-king-code/unauthority-core/releases)**

| Platform | File |
|----------|------|
| macOS | `LOS-Validator-*-macos.dmg` |
| Windows | `LOS-Validator-*-windows-x64.zip` |
| Linux | `LOS-Validator-*-linux-x64.tar.gz` |

### Platform Notes

> **macOS:** Remove quarantine: `xattr -cr /Applications/LOS\ Validator\ Node.app` (required for unsigned apps)  
> Or: System Settings → Privacy & Security → Open Anyway
>
> **Windows:** Right-click both `flutter_validator.exe` and `los-node.exe` → Properties → Unblock, then launch. If SmartScreen appears: click "More info" → "Run anyway"  
>
> **Linux:** Make executable: `chmod +x run.sh los-validator-miner los-node`, then run via `./run.sh` (sets `LD_LIBRARY_PATH` for native library).
>
> **First Launch:** The dashboard auto-downloads Tor Expert Bundle (~20MB, 1-2 min).
>
> **Bundled Binary:** The validator includes `los-node` (full validator binary) — no separate installation needed. Click "START NODE" in the dashboard to launch.

---

## Build from Source

### Prerequisites

- Flutter 3.27+ (`flutter --version`)
- Rust 1.75+ (`rustc --version`)

### Steps

```bash
# 1. Build the Dilithium5 native library
cd native/los_crypto_ffi
cargo build --release
cd ../..

# 2. Get Flutter dependencies
flutter pub get

# 3. Build for your platform
flutter build macos --release    # macOS
flutter build linux --release    # Linux
flutter build windows --release  # Windows
```

---

## Running a Validator

The dashboard includes a bundled `los-node` binary — no separate installation needed.

**Quick Start:**
1. Open the validator dashboard
2. Import or generate validator keys
3. Click "**START NODE**" to launch the bundled binary
4. Register as a validator (requires **1,000 LOS** minimum stake)
5. Monitor consensus participation in the dashboard

### Connecting to a Remote Node

To connect to a remote node instead of the bundled binary:

1. Open the app → **Settings**
2. Enter your node endpoint (e.g., `http://<peer-onion-address>:3030`)
3. Click **Test Connection** → **Save**

> **Note:** The validator dashboard always connects to **external peers** for API data — it never uses its own local node endpoint. This prevents a compromised local node from displaying false information.

---

## Project Structure

```
flutter_validator/
├── lib/
│   ├── main.dart              # App entry point
│   ├── constants/             # API URLs, theme colors
│   ├── models/                # Data models
│   ├── screens/               # Dashboard, settings, etc.
│   ├── services/              # API, wallet, Dilithium5, Tor, peer discovery
│   └── widgets/               # Reusable UI components
├── native/
│   └── los_crypto_ffi/        # Rust FFI crate for Dilithium5
└── test/                      # Widget & unit tests
```

---

## Related Documentation

- [**Flutter Validator Tutorial**](../docs/FLUTTER_VALIDATOR_GUIDE.md) — Step-by-step guide for this app
- [Validator Guide (CLI)](../docs/VALIDATOR_GUIDE.md) — Terminal-based setup for servers/VPS
- [API Reference](../docs/API_REFERENCE.md) — REST API endpoint documentation
- [Tor Setup](../docs/TOR_SETUP.md) — Tor hidden service configuration
- [Architecture](../docs/ARCHITECTURE.md) — System design overview

---

## License

AGPL-3.0 — See [LICENSE](../LICENSE)
