# Unauthority (LOS) — Genesis Wallet Transparency Report

> **Full transparency. Zero hidden wallets. Verify everything on-chain.**

This document publicly discloses **every wallet address** created at genesis for the Unauthority (LOS) blockchain. Anyone can independently verify balances using the public API.

---

## Total Supply

| Metric | Amount (LOS) | Amount (CIL) | % of Supply |
|--------|-------------|---------------|-------------|
| **Total Supply** | **21,936,236 LOS** | 2,193,623,600,000,000,000 CIL | 100% |
| Dev Treasury (4 wallets) | 773,823 LOS | 77,382,300,000,000,000 CIL | ~3.53% |
| Bootstrap Validator Stake (4 nodes) | 4,000 LOS | 400,000,000,000,000 CIL | ~0.018% |
| **Public Mining Pool** | **21,158,413 LOS** | 2,115,841,300,000,000,000 CIL | **~96.45%** |

> **Note:** 1 LOS = 10^11 CIL (atomic unit). Bootstrap validators receive **ZERO** mining rewards and **ZERO** epoch rewards — all rewards go to public participants.

---

## Bootstrap Validator Addresses (Stake Only — ZERO Rewards)

These 4 validators bootstrap the network. They hold **1,000 LOS each** (minimum stake) and are **code-level blocked** from receiving any mining or validator rewards.

| # | Address | Stake (LOS) | Onion Address | REST Port |
|---|---------|-------------|---------------|-----------|
| V1 | `LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1` | 1,000 | `kljkjqozqois4hgzz66kdmggmuneggfw3zm7sa76vk7fmoz7pie5kyad.onion` | 3030 |
| V2 | `LOSX2zcWmFPwowrvTiyqHxcndk5UrK8vJyNDK` | 1,000 | `cpdtxc3q3kt6krhx46ljnjnzswyu62p4sspeulagwu2schflaekaegqd.onion` | 3031 |
| V3 | `LOSWtyPmdxEiah9TN5GwARUKgvqpiLfXof9yq` | 1,000 | `yxqhqpwun6y7qhsboho7fkcso2hgp7lq3orfxdemuwm6iu5tfrfhvdad.onion` | 3032 |
| V4 | `LOSWoNusVctuR9TJKtpWa8fZdisdWk3XgznML` | 1,000 | `pqt2k7dspuyby7krdcfp2dv2ynb4hvliqyx5fcwnfmnnxvprnn4gsbad.onion` | 3033 |

---

## Dev Treasury Addresses

Transparent development fund wallets. These are the **only** non-public wallets in the entire genesis.

| # | Label | Address | Balance (LOS) | % of Supply |
|---|-------|---------|--------------|-------------|
| 1 | Dev Treasury 1 | `LOSX8EQkSAj1agD38gBuQnefLcHiVuCNQX17d` | 428,113 LOS | ~1.95% |
| 2 | Dev Treasury 2 | `LOSWsgUiweUN3FgcojRUFdhVgR2pteTqQyPX4` | 245,710 LOS | ~1.12% |
| 3 | Dev Treasury 3 | `LOSWooqdnLpFZZewiVkiieQUsndsGSWoiYHCK` | 50,000 LOS | ~0.23% |
| 4 | Dev Treasury 4 | `LOSX9fSrsbAZVT4bQXoYULdARJ8KrCqV14aka` | 50,000 LOS | ~0.23% |
| | **Total Dev Treasury** | | **773,823 LOS** | **~3.53%** |

---

## How to Verify Balances (Public API)

Anyone can verify any wallet balance using our public REST API. **No authentication required.**

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /bal/{address}` | Quick balance check |
| `GET /balance/{address}` | Alias for `/bal/{address}` |
| `GET /account/{address}` | Full account details (balance, blocks, validator status) |

### Via Tor (.onion) — Recommended

Use any of the 4 bootstrap validator nodes to query balances through Tor:

```bash
# Check Dev Treasury 1 balance via Tor
curl --socks5-hostname 127.0.0.1:9050 \
  http://kljkjqozqois4hgzz66kdmggmuneggfw3zm7sa76vk7fmoz7pie5kyad.onion:3030/bal/LOSX8EQkSAj1agD38gBuQnefLcHiVuCNQX17d

# Check Dev Treasury 2 balance via Tor
curl --socks5-hostname 127.0.0.1:9050 \
  http://cpdtxc3q3kt6krhx46ljnjnzswyu62p4sspeulagwu2schflaekaegqd.onion:3031/bal/LOSWsgUiweUN3FgcojRUFdhVgR2pteTqQyPX4

# Check Bootstrap Validator 1 stake
curl --socks5-hostname 127.0.0.1:9050 \
  http://yxqhqpwun6y7qhsboho7fkcso2hgp7lq3orfxdemuwm6iu5tfrfhvdad.onion:3032/bal/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1
```

> **Prerequisite:** You need Tor running locally (SOCKS5 proxy on port 9050). Install via `brew install tor && tor` (macOS) or `sudo apt install tor && sudo systemctl start tor` (Linux).

### Via Clearnet (If node exposes IP/domain)

If any validator runs on clearnet, you can query directly:

```bash
# Replace <NODE_IP> and <PORT> with the node's public address
curl http://<NODE_IP>:<PORT>/bal/LOSX8EQkSAj1agD38gBuQnefLcHiVuCNQX17d
```

### Example Response

```json
{
  "address": "LOSX8EQkSAj1agD38gBuQnefLcHiVuCNQX17d",
  "balance_cil": 42811300000000000,
  "balance_cil_str": "42811300000000000",
  "balance_los": "428113.00000000000",
  "block_count": 0,
  "head": "0"
}
```

### Understanding the Response

| Field | Description |
|-------|-------------|
| `address` | The LOS wallet address |
| `balance_cil` | Balance in CIL (atomic unit, integer) |
| `balance_los` | Balance in LOS (human-readable) |
| `block_count` | Number of blocks (transactions) for this account |
| `head` | Hash of the latest block in this account's chain |

### Full Account Details

For more information including validator status:

```bash
curl --socks5-hostname 127.0.0.1:9050 \
  http://kljkjqozqois4hgzz66kdmggmuneggfw3zm7sa76vk7fmoz7pie5kyad.onion:3030/account/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1
```

---

## Verify All Genesis Wallets (Quick Script)

Copy-paste this script to verify **every** genesis wallet in one go:

```bash
#!/bin/bash
# Verify all Unauthority genesis wallets via Tor
# Requires: tor running on localhost:9050

NODE="kljkjqozqois4hgzz66kdmggmuneggfw3zm7sa76vk7fmoz7pie5kyad.onion:3030"
PROXY="--socks5-hostname 127.0.0.1:9050"

echo "============================================="
echo " Unauthority (LOS) Genesis Wallet Audit"
echo "============================================="
echo ""

echo "--- BOOTSTRAP VALIDATORS (1,000 LOS each, ZERO rewards) ---"
for addr in \
  LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1 \
  LOSX2zcWmFPwowrvTiyqHxcndk5UrK8vJyNDK \
  LOSWtyPmdxEiah9TN5GwARUKgvqpiLfXof9yq \
  LOSWoNusVctuR9TJKtpWa8fZdisdWk3XgznML
do
  echo -n "[$addr] -> "
  curl -s $PROXY http://$NODE/bal/$addr | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f\"{d['balance_los']} LOS ({d['balance_cil']} CIL)\")
"
done

echo ""
echo "--- DEV TREASURY WALLETS ---"
for addr in \
  LOSX8EQkSAj1agD38gBuQnefLcHiVuCNQX17d \
  LOSWsgUiweUN3FgcojRUFdhVgR2pteTqQyPX4 \
  LOSWooqdnLpFZZewiVkiieQUsndsGSWoiYHCK \
  LOSX9fSrsbAZVT4bQXoYULdARJ8KrCqV14aka
do
  echo -n "[$addr] -> "
  curl -s $PROXY http://$NODE/bal/$addr | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f\"{d['balance_los']} LOS ({d['balance_cil']} CIL)\")
"
done

echo ""
echo "============================================="
echo " Total Non-Public: 777,823 LOS (~3.55%)"
echo " Total Public (Mining Pool): 21,158,413 LOS (~96.45%)"
echo " Total Supply: 21,936,236 LOS (Fixed)"
echo "============================================="
```

---

## Why This Matters

1. **No hidden wallets** — Every genesis address is published here.
2. **On-chain verifiable** — Anyone can query any node to confirm balances.
3. **Fair distribution** — 96.45% of supply goes to public PoW miners.
4. **Bootstrap integrity** — Genesis validators get stake only, zero rewards.
5. **Immutable record** — This genesis config is baked into the binary at build time.

---

## Security Notice

- **Private keys and seed phrases are NOT included** in this document or in `genesis_config.json`.
- All private keys are stored offline, air-gapped, and backed up separately.
- The `genesis_config.json` file only contains public keys and addresses.

---

*Last updated: 2026-02-26 — Genesis Block*
