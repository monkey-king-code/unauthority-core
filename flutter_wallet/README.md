# LOS Wallet

Desktop wallet for the **Unauthority (LOS)** blockchain. Send, receive, and burn-to-mint LOS tokens with post-quantum security.

[![Version](https://img.shields.io/badge/version-2.0.0-blue)]()
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](../LICENSE)

---

## Features

- **Create / Import Wallet** — generate new keys or recover from 24-word BIP39 seed phrase
- **Send & Receive LOS** — instant transactions with < 3 second finality
- **Burn LOS** — burn LOS tokens with validator consensus
- **Address Book** — save frequently used addresses
- **Transaction History** — view all past transactions
- **QR Code** — share your address via QR
- **Built-in Tor** — auto-downloads Tor Expert Bundle (no Tor Browser needed)
- **CRYSTALS-Dilithium5** — post-quantum digital signatures via native Rust FFI
- **Multi-Platform** — macOS (Intel + Apple Silicon), Linux, Windows

---

## Download

Pre-built releases for macOS, Windows, and Linux:

**[Download from GitHub Releases](https://github.com/monkey-king-code/unauthority-core/releases)**

| Platform | File |
|----------|------|
| macOS | `LOS-Wallet-*-macos.dmg` |
| Windows | `LOS-Wallet-*-windows-x64.zip` |
| Linux | `LOS-Wallet-*-linux-x64.tar.gz` |

### Platform Notes

> **macOS:** Remove quarantine: `xattr -cr /Applications/LOS\ Wallet.app` (required for unsigned apps)  
> Or: System Settings → Privacy & Security → Open Anyway
>
> **Windows:** Right-click `flutter_wallet.exe` → Properties → Unblock, then launch. If SmartScreen appears: click "More info" → "Run anyway"  
>
> **Linux:** Make executable: `chmod +x run.sh flutter_wallet`, then run via `./run.sh` (sets `LD_LIBRARY_PATH` for native library).
>
> **First Launch:** The wallet auto-downloads Tor Expert Bundle (~20MB, 1-2 min).

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

The native library (`liblos_crypto_ffi.dylib` / `.so` / `.dll`) must be placed alongside the built app. See the GitHub Actions workflow for platform-specific bundling steps.

---

## Connecting to the Network

The wallet **auto-connects** to mainnet peers via Tor on first launch. No manual configuration is required — Tor downloads automatically.

### Manual Configuration (Optional)

If auto-discovery fails:

1. Open the app → **Settings** tab
2. Enter a peer endpoint manually (e.g., `http://<peer-onion-address>:3030`)
3. Click **Test Connection** → **Save & Reconnect**

---

## Project Structure

```
flutter_wallet/
├── lib/
│   ├── main.dart              # App entry point
│   ├── constants/             # API URLs, theme colors
│   ├── models/                # Data models
│   ├── screens/               # UI screens (dashboard, send, burn, etc.)
│   ├── services/              # API, wallet, Dilithium5, Tor, peer discovery
│   ├── utils/                 # Formatting helpers
│   └── widgets/               # Reusable UI components
├── native/
│   └── los_crypto_ffi/        # Rust FFI crate for Dilithium5
├── assets/                    # Icons, images, network config
└── test/                      # Widget & unit tests
```

---

## Related Documentation

- [Validator Guide](../docs/VALIDATOR_GUIDE.md) — Run a validator node
- [API Reference](../docs/API_REFERENCE.md) — REST API endpoint documentation
- [Tor Setup](../docs/TOR_SETUP.md) — Tor hidden service configuration
- [Architecture](../docs/ARCHITECTURE.md) — System design overview

---

## License

AGPL-3.0 — See [LICENSE](../LICENSE)
