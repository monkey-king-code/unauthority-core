# Exchange Integration Guide — Unauthority (LOS) v1.0.13

Complete RPC documentation for cryptocurrency exchanges, custodians, and payment processors integrating LOS.

*Last updated: February 2026*

---

## Table of Contents

1. [Overview](#overview)
2. [Network Information](#network-information)
3. [Connection Setup](#connection-setup)
4. [Key Concepts](#key-concepts)
5. [Core Endpoints for Exchanges](#core-endpoints-for-exchanges)
6. [Deposit Detection](#deposit-detection)
7. [Withdrawal Processing](#withdrawal-processing)
8. [Balance Monitoring](#balance-monitoring)
9. [Transaction Confirmation](#transaction-confirmation)
10. [Supply & Market Data](#supply--market-data)
11. [Health & Status Monitoring](#health--status-monitoring)
12. [Error Handling](#error-handling)
13. [Security Considerations](#security-considerations)
14. [Rate Limits](#rate-limits)
15. [Code Examples](#code-examples)

---

## Overview

Unauthority (LOS) is a block-lattice (DAG) blockchain using post-quantum Dilithium5 cryptography. The network operates exclusively over Tor hidden services (`.onion` addresses). The native currency is **LOS** with atomic unit **CIL** (1 LOS = 10^11 CIL).

| Property | Value |
|---|---|
| **Ticker** | LOS |
| **Atomic Unit** | CIL |
| **Precision** | 11 decimal places (1 LOS = 100,000,000,000 CIL) |
| **Total Supply** | 21,936,236 LOS (fixed, non-inflationary) |
| **Block Time** | ~2-3 seconds (finality) |
| **Network** | Tor hidden services (.onion) exclusively |
| **Cryptography** | CRYSTALS-Dilithium5 (Post-Quantum, NIST FIPS 204) |
| **API Protocol** | REST (JSON) and gRPC (Protocol Buffers) |
| **Address Format** | Base58, prefix `LOS` (e.g., `LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`) |

---

## Network Information

### Mainnet

| Parameter | Value |
|---|---|
| **Chain ID** | `los-mainnet` |
| **Network ID** | 1 |
| **Default REST Port** | 3030 |
| **Default P2P Port** | 4030 |
| **Default gRPC Port** | 23030 |
| **Genesis Validators** | 4 |

### Bootstrap Nodes

These are the genesis validator `.onion` addresses for initial peer discovery:

| Validator | Onion Address | REST Port |
|---|---|---|
| V1 | `f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion` | 3030 |
| V2 | `xchdcoebass6ewt7astm2ksacr55s7nd6l74qcmvkrm37t7pnqqf32qd.onion` | 3031 |
| V3 | `7v5lqrgevfeyhb6aomt75dbagnkmxrthg6qehkxa2fv5ycxxam7e25qd.onion` | 3032 |
| V4 | `7dbvneima7h5nc34x2c7frgyq64h64ipbes43pkd4lmc7hnfxzwl3lyd.onion` | 3033 |

---

## Connection Setup

### Tor SOCKS5 Proxy

All connections to LOS nodes require a Tor SOCKS5 proxy. Install Tor and use the SOCKS5 proxy at `127.0.0.1:9050`.

**Example (curl):**
```bash
curl --socks5-hostname 127.0.0.1:9050 \
  http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030/health
```

**Example (Python):**
```python
import requests

proxies = {
    'http': 'socks5h://127.0.0.1:9050',
    'https': 'socks5h://127.0.0.1:9050',
}

response = requests.get(
    'http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030/health',
    proxies=proxies,
    timeout=30
)
print(response.json())
```

**Example (Node.js):**
```javascript
const { SocksProxyAgent } = require('socks-proxy-agent');
const axios = require('axios');

const agent = new SocksProxyAgent('socks5h://127.0.0.1:9050');
const BASE_URL = 'http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030';

const response = await axios.get(`${BASE_URL}/health`, { httpAgent: agent });
console.log(response.data);
```

### Failover Strategy

1. Query all 4 bootstrap nodes to find active peers
2. Use `GET /peers` on any responding node to discover additional nodes
3. Implement round-robin or latency-based selection across multiple nodes
4. Tor adds 500ms–2s latency per request — set timeouts accordingly (recommended: 30s)

---

## Key Concepts

### Block-Lattice Architecture

Unlike traditional blockchains, LOS uses a per-account chain model. Each account has its own chain of blocks. Transactions are:
- **Send**: Debit from the sender's account chain
- **Receive**: Credit to the receiver's account chain

A complete transfer requires both a Send and a Receive block.

### Amounts

All amounts in the API are returned as **string representations of LOS** (human-readable) unless otherwise noted. Internally, the system uses CIL (atomic units) as `u128` integers.

```
1 LOS = 100,000,000,000 CIL (10^11)
```

### Addresses

LOS addresses are Base58-encoded SHA-3 hashes of Dilithium5 public keys:
- Always start with `LOS` prefix
- Approximately 36 characters long
- Example: `LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`

---

## Core Endpoints for Exchanges

### Endpoint Summary

| Priority | Method | Endpoint | Purpose |
|---|---|---|---|
| **Critical** | GET | `/bal/{address}` | Check account balance |
| **Critical** | GET | `/account/{address}` | Full account details |
| **Critical** | GET | `/history/{address}` | Transaction history (deposit detection) |
| **Critical** | POST | `/send` | Send LOS (withdrawals) |
| **Critical** | GET | `/supply` | Total/circulating supply |
| **Critical** | GET | `/health` | Node health check |
| Important | GET | `/tx/{hash}` | Transaction (block) lookup by hash |
| Important | GET | `/validators` | Active validator list |
| Important | GET | `/consensus` | Consensus safety status |
| Important | GET | `/peers` | Connected peer information |
| Important | GET | `/node-info` | Node version and status |
| Utility | GET | `/block` | Latest block |
| Utility | GET | `/blocks/recent` | Recent blocks |
| Utility | GET | `/metrics` | Prometheus metrics |
| Utility | GET | `/fee-info` | Current fee information |

---

## Deposit Detection

### Method: Poll Transaction History

Poll `GET /history/{address}` periodically to detect incoming deposits.

**Request:**
```
GET /history/{deposit_address}
```

**Response:**
```json
{
  "status": "ok",
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "history": [
    {
      "hash": "a1b2c3d4e5f6...",
      "block_type": "Receive",
      "amount": "50.00000000000",
      "link": "f6e5d4c3b2a1...",
      "timestamp": 1771277598,
      "previous": "0000000000..."
    },
    {
      "hash": "b2c3d4e5f6a1...",
      "block_type": "Send",
      "amount": "10.00000000000",
      "link": "LOSWsgUiweUN3FgcojRUFdhVgR2pteTqQyPX4",
      "timestamp": 1771277650,
      "previous": "a1b2c3d4e5f6..."
    }
  ]
}
```

**Deposit detection logic:**
1. Poll `/history/{address}` every 10-30 seconds
2. Filter for `block_type: "Receive"` entries
3. Track previously seen block hashes to identify new deposits
4. A `Receive` block with a new hash = confirmed deposit

### Method: Account Balance Polling

For simpler integration, monitor balance changes:

**Request:**
```
GET /bal/{deposit_address}
```

**Response:**
```json
{
  "status": "ok",
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "balance": "1050.00000000000",
  "balance_cil": 105000000000000,
  "pending_receives": 0
}
```

### Confirmation Model

LOS uses aBFT consensus — once a block appears in the account history, it is **finalized and irreversible**. There is no concept of block confirmations or chain reorganizations.

| Confirmation Status | Meaning |
|---|---|
| Block in history | **Final** — irreversible |
| Consensus reached | 2f+1 validators agreed (f = max faulty) |

**Recommended:** 1 confirmation is sufficient. Once a Receive block appears in `/history/{address}`, the deposit is final.

---

## Withdrawal Processing

### Send Transaction

**Request:**
```
POST /send
Content-Type: application/json

{
  "from": "LOSX_EXCHANGE_HOT_WALLET_ADDRESS",
  "target": "LOSWcustomer_withdrawal_address",
  "amount": "100.5",
  "public_key": "HEX_DILITHIUM5_PUBLIC_KEY",
  "signature": "HEX_DILITHIUM5_SIGNATURE"
}
```

**Response (Success):**
```json
{
  "status": "ok",
  "msg": "Transaction processed",
  "block_hash": "a1b2c3d4e5f6...",
  "from": "LOSX_EXCHANGE_HOT_WALLET_ADDRESS",
  "to": "LOSWcustomer_withdrawal_address",
  "amount": "100.5",
  "fee": "0.00001000000",
  "new_balance": "9899.49999000000"
}
```

**Response (Error):**
```json
{
  "status": "error",
  "msg": "Insufficient balance",
  "code": 400
}
```

### Signing Transactions

LOS uses Dilithium5 (post-quantum) signatures. The signing payload is:

```
message = from + target + amount_cil_string
signature = dilithium5_sign(secret_key, message)
```

Where `amount_cil_string` is the amount converted to CIL as a string (e.g., `"10050000000000"` for 100.5 LOS).

### Fee Structure

Transaction fees are dynamic, based on network congestion:

```
GET /fee-info
```

```json
{
  "status": "ok",
  "base_fee_cil": 1000000,
  "current_multiplier": 1,
  "effective_fee_cil": 1000000,
  "effective_fee_los": "0.00001000000"
}
```

- **Base fee:** 0.00001 LOS (1,000,000 CIL)
- **Scaling:** Exponential (2× per tx above threshold of 5 tx/minute per account)
- Fees are automatically calculated and deducted

---

## Balance Monitoring

### Single Account Balance

```
GET /bal/{address}
```

```json
{
  "status": "ok",
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "balance": "1000.00000000000",
  "balance_cil": 100000000000000,
  "pending_receives": 0
}
```

### Full Account Details

```
GET /account/{address}
```

```json
{
  "status": "ok",
  "account": {
    "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
    "balance": "1000.00000000000",
    "balance_cil": 100000000000000,
    "block_count": 5,
    "head_hash": "a1b2c3d4e5f6...",
    "public_key": "ea465632b8a25f09..."
  }
}
```

---

## Transaction Confirmation

### Lookup by Block Hash

```
GET /tx/{block_hash}
```

```json
{
  "status": "ok",
  "block": {
    "hash": "a1b2c3d4e5f6...",
    "account": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
    "block_type": "Send",
    "amount": "100.50000000000",
    "link": "LOSWcustomer_address",
    "previous": "f6e5d4c3b2a1...",
    "timestamp": 1771277598,
    "signature": "d4e5f6...",
    "public_key": "ea465632..."
  }
}
```

### Block Finality

LOS blocks are **immediately final** once they appear in the ledger. The aBFT consensus protocol ensures:
- No forks or chain reorganizations
- A block committed to the ledger cannot be reversed
- Liveness requires ≥2/3 validators online

**For exchanges:** Set required confirmations to **1**. There is no need for multiple confirmations.

---

## Supply & Market Data

### Total Supply

```
GET /supply
```

```json
{
  "status": "ok",
  "total_supply": "21936236.00000000000",
  "total_supply_cil": 2193623600000000000,
  "circulating_supply": "777823.00000000000",
  "circulating_supply_cil": 77782300000000000,
  "burned_supply": "0.00000000000",
  "burned_supply_cil": 0,
  "remaining_public_supply": "21158413.00000000000",
  "remaining_public_supply_cil": 2115841300000000000
}
```

| Field | Description |
|---|---|
| `total_supply` | Fixed total: 21,936,236 LOS |
| `circulating_supply` | Currently distributed tokens |
| `burned_supply` | Tokens destroyed via contract burns |
| `remaining_public_supply` | Remaining tokens available through PoW mining |

### Reward Pool Info

```
GET /reward-info
```

```json
{
  "status": "ok",
  "total_pool": "500000.00000000000",
  "distributed": "0.00000000000",
  "remaining": "500000.00000000000",
  "current_epoch": 0,
  "epoch_reward_rate": "5000.00000000000",
  "epoch_duration_seconds": 2592000,
  "next_halving_epoch": 48
}
```

---

## Health & Status Monitoring

### Health Check

```
GET /health
```

```json
{
  "status": "healthy",
  "version": "1.0.13",
  "timestamp": 1771277598,
  "uptime_seconds": 86400,
  "chain": {
    "accounts": 8,
    "blocks": 42,
    "id": "los-mainnet"
  },
  "database": {
    "accounts_count": 8,
    "blocks_count": 42,
    "size_on_disk": 524287
  }
}
```

### Node Information

```
GET /node-info
```

```json
{
  "status": "ok",
  "version": "1.0.13",
  "network": "mainnet",
  "node_id": "validator-1",
  "chain_id": "los-mainnet",
  "peer_count": 3,
  "block_count": 42,
  "account_count": 8,
  "uptime_seconds": 86400
}
```

### Consensus Status

```
GET /consensus
```

```json
{
  "status": "ok",
  "view": 42,
  "leader": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "validators": 4,
  "quorum_required": 3,
  "max_faulty": 1,
  "safety": "3f < n satisfied",
  "pending_blocks": 0
}
```

### Prometheus Metrics

```
GET /metrics
```

Returns Prometheus-compatible text format with these key metrics:

| Metric | Type | Description |
|---|---|---|
| `los_blocks_total` | Counter | Total blocks processed |
| `los_accounts_total` | Gauge | Total accounts |
| `los_active_validators` | Gauge | Active validators |
| `los_peer_count` | Gauge | Connected peers |
| `los_uptime_seconds` | Gauge | Node uptime |
| `los_consensus_rounds` | Counter | aBFT rounds completed |

---

## Error Handling

### Error Response Format

```json
{
  "status": "error",
  "msg": "Description of the error",
  "code": 400
}
```

### Common Error Codes

| HTTP Status | Error | Meaning |
|---|---|---|
| 400 | `"Invalid address format"` | Address doesn't start with `LOS` or is malformed |
| 400 | `"Insufficient balance"` | Sender doesn't have enough LOS |
| 400 | `"Invalid signature"` | Dilithium5 signature verification failed |
| 400 | `"Amount must be positive"` | Zero or negative amount |
| 404 | `"Account not found"` | Address has never received any LOS |
| 429 | `"Rate limit exceeded"` | Too many requests from this IP |
| 500 | `"Internal server error"` | Node-side error (retry with backoff) |

### Recommended Error Handling

```python
def send_withdrawal(to_address, amount):
    try:
        response = requests.post(f"{BASE_URL}/send", json={
            "from": HOT_WALLET, "target": to_address,
            "amount": str(amount), "public_key": PUB_KEY,
            "signature": sign(HOT_WALLET + to_address + to_cil(amount))
        }, proxies=TOR_PROXIES, timeout=30)

        data = response.json()
        if data["status"] == "ok":
            return {"success": True, "tx_hash": data["block_hash"]}
        else:
            return {"success": False, "error": data["msg"]}

    except requests.Timeout:
        # Tor latency — retry with exponential backoff
        return {"success": False, "error": "timeout", "retry": True}
    except Exception as e:
        return {"success": False, "error": str(e), "retry": True}
```

---

## Security Considerations

### Tor Connectivity

- All communication MUST go through Tor SOCKS5 proxy
- Never expose validator `.onion` addresses alongside clearnet infrastructure
- Set appropriate timeouts (30s recommended for Tor)
- Implement connection retry logic with exponential backoff

### Transaction Signing

- Keep Dilithium5 private keys in HSM or secure cold storage
- Sign withdrawal transactions offline when possible
- Verify signatures before broadcasting
- Public keys are ~2.5 KB, signatures ~4.6 KB — larger than ECDSA

### Address Validation

```python
def is_valid_los_address(address: str) -> bool:
    """Validate LOS address format."""
    if not address.startswith("LOS"):
        return False
    if len(address) < 30 or len(address) > 45:
        return False
    # Base58 character set check
    valid_chars = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    return all(c in valid_chars for c in address[3:])
```

### Hot Wallet Best Practices

1. **Minimum balance** — Keep only required withdrawal float in hot wallet
2. **Rate limiting** — Implement internal withdrawal rate limits
3. **Anomaly detection** — Monitor for unusual withdrawal patterns
4. **Multi-signature** — Consider a multi-step withdrawal approval process
5. **Backup keys** — BIP39 seed phrase must be securely backed up offline

---

## Rate Limits

| Endpoint Category | Limit | Window |
|---|---|---|
| Read-only (GET) | 100 requests | 60 seconds |
| State-changing (POST) | 10 requests | 60 seconds |
| Per-address transactions | 5 sends | 60 seconds |

Rate limit responses return HTTP 429:
```json
{ "status": "error", "msg": "Rate limit exceeded", "code": 429 }
```

---

## Code Examples

### Python: Full Deposit Monitor

```python
import time
import requests

PROXIES = {'http': 'socks5h://127.0.0.1:9050'}
NODE = 'http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030'
DEPOSIT_ADDRESS = 'LOSX_YOUR_DEPOSIT_ADDRESS'

seen_hashes = set()

def check_deposits():
    """Poll for new deposits."""
    try:
        resp = requests.get(
            f'{NODE}/history/{DEPOSIT_ADDRESS}',
            proxies=PROXIES, timeout=30
        )
        data = resp.json()
        if data['status'] != 'ok':
            return []

        new_deposits = []
        for tx in data.get('history', []):
            if tx['block_type'] == 'Receive' and tx['hash'] not in seen_hashes:
                seen_hashes.add(tx['hash'])
                new_deposits.append({
                    'hash': tx['hash'],
                    'amount': tx['amount'],
                    'timestamp': tx['timestamp']
                })
        return new_deposits
    except Exception as e:
        print(f'Error: {e}')
        return []

# Main loop
while True:
    deposits = check_deposits()
    for d in deposits:
        print(f"New deposit: {d['amount']} LOS (tx: {d['hash'][:16]}...)")
        # Credit user account in your database
    time.sleep(15)  # Poll every 15 seconds
```

### Node.js: Balance Checker

```javascript
const { SocksProxyAgent } = require('socks-proxy-agent');
const axios = require('axios');

const agent = new SocksProxyAgent('socks5h://127.0.0.1:9050');
const NODE = 'http://f3zfmhvverdljhddhxvdnkibrajd2cbolrfq4z6a5y2ifprf2xh34nid.onion:3030';

async function getBalance(address) {
  try {
    const res = await axios.get(`${NODE}/bal/${address}`, {
      httpAgent: agent,
      timeout: 30000,
    });
    return {
      address: res.data.address,
      balance: res.data.balance,
      balance_cil: res.data.balance_cil,
    };
  } catch (err) {
    console.error(`Balance check failed: ${err.message}`);
    return null;
  }
}

// Usage
const balance = await getBalance('LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1');
console.log(`Balance: ${balance.balance} LOS`);
```

---

## Additional Resources

- [API Reference](API_REFERENCE.md) — Complete endpoint documentation with all parameters
- [Whitepaper](WHITEPAPER.md) — Technical details on consensus, economics, and security
- [Tor Setup](TOR_SETUP.md) — Detailed Tor configuration guide
- [Architecture](ARCHITECTURE.md) — System design and data flow

---

*Unauthority (LOS) — Lattice Of Sovereignty*  
*100% Immutable. 100% Permissionless. 100% Decentralized.*
