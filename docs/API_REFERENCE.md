# API Reference — Unauthority (LOS) v2.0.1

Complete REST API and gRPC API documentation for the `los-node` validator binary.

---

## Base URL

| Protocol | Address | Notes |
|---|---|---|
| **REST** | `http://127.0.0.1:3030` | Default port, configurable via `--port` |
| **gRPC** | `127.0.0.1:23030` | Always REST port + 20,000 |
| **Tor** | `http://YOUR_ONION.onion:3030` | Via SOCKS5 proxy |

## Authentication

No authentication required. Rate limiting is enforced per IP for state-changing endpoints.

## Error Format

All errors return:
```json
{ "status": "error", "msg": "Description of the error", "code": 400 }
```

---

## Table of Contents

- [Status Endpoints](#status-endpoints)
- [Account Endpoints](#account-endpoints)
- [Block Endpoints](#block-endpoints)
- [Transaction Endpoints](#transaction-endpoints)
- [Validator Endpoints](#validator-endpoints)
- [Consensus & Oracle](#consensus--oracle)
- [Smart Contract Endpoints](#smart-contract-endpoints)
- [Network Endpoints](#network-endpoints)
- [Utility Endpoints](#utility-endpoints)
- [gRPC API](#grpc-api)
- [USP-01 Token Endpoints](#usp-01-token-endpoints)
- [DEX AMM Endpoints](#dex-amm-endpoints)
- [CLI Reference](#cli-reference)
- [Rate Limits](#rate-limits)

---

## Status Endpoints

### GET `/`

Node status overview with all available endpoints.

**Response:**
```json
{
  "name": "Unauthority (LOS) Blockchain API",
  "version": "2.0.1",
  "network": "mainnet",
  "status": "operational",
  "description": "Decentralized blockchain with aBFT consensus",
  "endpoints": {
    "health": "GET /health - Health check",
    "supply": "GET /supply - Total supply, burned, remaining",
    "bal": "GET /bal/{address} - Account balance (short alias)",
    "send": "POST /send {from, target, amount} - Send transaction",
    "...": "..."
  }
}
```

### GET `/health`

Health check for monitoring and load balancing.

**Response:**
```json
{
  "status": "healthy",
  "version": "2.0.1",
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

### GET `/node-info`

Detailed node information.

**Response:**
```json
{
  "node_id": "validator-1",
  "version": "2.0.1",
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "block_count": 42,
  "account_count": 8,
  "peers": 4,
  "is_validator": true,
  "uptime_seconds": 86400,
  "network": "mainnet"
}
```

### GET `/supply`

Total, circulating, and burned supply information.

**Response:**
```json
{
  "total_supply": "21936236.00000000000",
  "total_supply_cil": 2193623600000000000,
  "circulating_supply": "777823.00000000000",
  "circulating_supply_cil": 77782300000000000,
  "remaining_supply": "21158413.00000000000",
  "remaining_supply_cil": 2115841300000000000,
  "total_burned_usd": 0
}
```

### GET `/metrics`

Prometheus-compatible metrics output.

**Response:** (text/plain)
```
# HELP los_blocks_total Total blocks in ledger
los_blocks_total 42
# HELP los_accounts_total Total accounts
los_accounts_total 8
# HELP los_active_validators Active validator count
los_active_validators 4
# HELP los_peer_count Connected peers
los_peer_count 4
# HELP los_consensus_rounds aBFT consensus rounds
los_consensus_rounds 128
# HELP los_uptime_seconds Node uptime
los_uptime_seconds 86400
```

---

## Account Endpoints

### GET `/bal/{address}`

Get account balance. Returns balance in both CIL (atomic unit) and LOS.

**Example:** `GET /bal/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`

**Response:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "balance_cil": 100000000000000,
  "balance_cil_str": "100000000000000",
  "balance_los": "1000.00000000000",
  "block_count": 0,
  "head": "0"
}
```

### GET `/balance/{address}`

Alias for `/bal/{address}`. Same response format.

### GET `/account/{address}`

Full account details including balance, block count, validator status, and recent transaction history.

**Example:** `GET /account/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`

**Response:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "balance_cil": 100000000000000,
  "balance_los": "1000.00000000000",
  "block_count": 5,
  "head": "abc123...",
  "is_validator": true,
  "stake_cil": 100000000000000,
  "recent_blocks": [ ... ]
}
```

### GET `/history/{address}`

Transaction history for an address.

**Example:** `GET /history/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`

**Response:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "transactions": [
    {
      "hash": "abc123...",
      "type": "Send",
      "amount": 100000000000000,
      "from": "LOSX7dSt...",
      "to": "LOSWoNus...",
      "timestamp": 1771277598,
      "fee": 100000000
    }
  ]
}
```

### GET `/fee-estimate/{address}`

Estimate the transaction fee for an address. Returns the flat BASE_FEE_CIL.

**Example:** `GET /fee-estimate/LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1`

**Response:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "fee_cil": 100000000,
  "fee_los": "0.00100000000"
}
```

---

## Block Endpoints

### GET `/block`

Latest block across all accounts.

**Response:**
```json
{
  "account": "LOSX7dSt...",
  "previous": "def456...",
  "block_type": "Send",
  "amount": 50000000000000,
  "link": "LOSWoNus...",
  "hash": "abc123...",
  "timestamp": 1771277598,
  "height": 42
}
```

### GET `/block/{hash}`

Get a specific block by its SHA-3 hash.

**Example:** `GET /block/abc123def456...`

### GET `/blocks/recent`

Recent blocks (last 50).

**Response:**
```json
{
  "blocks": [ ... ],
  "count": 50
}
```

---

## Transaction Endpoints

### POST `/send`

Send LOS to another address.

#### Client-Signed Transaction (Recommended)

The client signs the transaction with Dilithium5. This is the secure method used by wallets.

**Request:**
```json
{
  "from": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "target": "LOSWoNusVctuR9TJKtpWa8fZdisdWk3XgznML",
  "amount": 10,
  "amount_cil": 1000000000000,
  "signature": "hex_dilithium5_signature...",
  "public_key": "hex_dilithium5_public_key...",
  "previous": "hash_of_previous_block...",
  "timestamp": 1771277598,
  "fee": 100000000
}
```

**Fields:**
- `from` — Sender address
- `target` — Recipient address
- `amount` — Amount in LOS (or use `amount_cil` for atomic units)
- `signature` — Dilithium5 hex signature over the transaction payload
- `public_key` — Sender's Dilithium5 hex public key
- `previous` — Hash of the sender's latest block (from `/bal/{address}`)
- `timestamp` — Unix timestamp
- `fee` — Fee in CIL (from `/fee-estimate`)

#### Node-Signed Transaction (Testnet/Development)

For testing, only `target` and `amount` are required. The node signs with its own key.

**Request:**
```json
{
  "target": "LOSWoNusVctuR9TJKtpWa8fZdisdWk3XgznML",
  "amount": 10
}
```

**Response (both modes):**
```json
{
  "status": "ok",
  "hash": "abc123def456...",
  "from": "LOSX7dSt...",
  "to": "LOSWoNus...",
  "amount_cil": 1000000000000,
  "fee_cil": 100000000,
  "block_type": "Send"
}
```

### GET `/transaction/{hash}`

Look up a transaction by its hash.

**Example:** `GET /transaction/abc123def456...`

### GET `/search/{query}`

Search across blocks, accounts, and transaction hashes.

**Example:** `GET /search/LOSX7dSt`

---

## Transaction: Burn Bridge

### POST `/burn`

Burn ETH or BTC to receive LOS.

**Request:**
```json
{
  "coin_type": "eth",
  "txid": "0xabc123def456...",
  "recipient_address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1"
}
```

**Fields:**
- `coin_type` — `"eth"` or `"btc"`
- `txid` — Transaction hash of the burn on the source chain
- `recipient_address` — LOS address to receive minted tokens

**Process:**
1. Submit the burn TXID
2. Multi-validator oracle consensus verifies the burn amount and price
3. LOS is minted to the recipient proportional to USD value burned
4. All arithmetic uses u128 integer math (prices in micro-USD, amounts in wei/satoshi)

**Response:**
```json
{
  "status": "pending",
  "txid": "0xabc123...",
  "msg": "Burn submitted. Awaiting oracle consensus."
}
```

### POST `/reset-burn-txid`

Reset a stuck burn TXID (testnet only — disabled on mainnet).

---

## Validator Endpoints

### GET `/validators`

List all active validators.

**Response:**
```json
{
  "validators": [
    {
      "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
      "active": true,
      "connected": true,
      "has_min_stake": true,
      "is_genesis": true,
      "onion_address": "f3zfmh...nid.onion",
      "stake": 1000,
      "uptime_percentage": 99
    }
  ]
}
```

### POST `/register-validator`

Register as a network validator. Requires Dilithium5 signature and ≥1 LOS balance. Reward eligibility requires ≥1,000 LOS.

**Request:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "public_key": "hex_dilithium5_public_key...",
  "signature": "hex_dilithium5_signature...",
  "endpoint": "your-onion-address.onion:3030"
}
```

### POST `/unregister-validator`

Remove yourself from the validator set.

**Request:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "public_key": "hex_dilithium5_public_key...",
  "signature": "hex_dilithium5_signature..."
}
```

---

## Consensus & Oracle

### GET `/consensus`

aBFT consensus engine status and safety parameters.

**Response:**
```json
{
  "safety": {
    "active_validators": 4,
    "byzantine_threshold": 1,
    "byzantine_safe": true,
    "consensus_model": "aBFT"
  },
  "round": {
    "current": 128,
    "decided": 127
  }
}
```

### GET `/reward-info`

Validator reward pool and epoch information.

**Response:**
```json
{
  "epoch": {
    "current_epoch": 5,
    "epoch_reward_rate_los": 5000
  },
  "pool": {
    "remaining_los": 475000,
    "total_distributed_los": 25000
  },
  "validators": {
    "eligible": 4,
    "total": 4
  }
}
```

### GET `/slashing`

Global slashing statistics.

### GET `/slashing/{address}`

Slashing profile for a specific validator address.

---

## Smart Contract Endpoints

### POST `/deploy-contract`

Deploy a WASM smart contract to the UVM.

**Request:**
```json
{
  "wasm_hex": "0061736d...",
  "deployer": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
  "signature": "hex_signature...",
  "public_key": "hex_public_key..."
}
```

### POST `/call-contract`

Execute a function on a deployed smart contract.

**Request:**
```json
{
  "contract_id": "contract_address_or_hash",
  "function": "transfer",
  "args": ["LOSX7dSt...", "1000"],
  "caller": "LOSX7dSt...",
  "signature": "hex_signature...",
  "public_key": "hex_public_key..."
}
```

### GET `/contract/{id}`

Get the state and info of a deployed contract.

### GET `/contracts`

List all deployed contracts.

---

## Network Endpoints

### GET `/peers`

Connected peers and validator endpoints.

**Response:**
```json
{
  "peer_count": 4,
  "peers": [
    {
      "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1",
      "is_validator": true,
      "onion_address": "f3zfmh...nid.onion",
      "self": true,
      "short_address": "los_X7dStdPk"
    }
  ],
  "validator_endpoint_count": 4,
  "validator_endpoints": [
    {
      "address": "LOSX7dSt...",
      "onion_address": "f3zfmh...nid.onion"
    }
  ]
}
```

### GET `/network/peers`

Network-level peer discovery with endpoint information.

### GET `/mempool/stats`

Current mempool statistics.

**Response:**
```json
{
  "pending_transactions": 0,
  "pending_burns": 0,
  "queued": 0
}
```

### GET `/sync`

GZIP-compressed ledger state for node synchronization. Use `?from={block_count}` for incremental sync.

### GET `/whoami`

This node's signing address.

**Response:**
```json
{
  "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1"
}
```

---

## Utility Endpoints

### GET `/tor-health`

Tor hidden service self-check status.

**Response:**
```json
{
  "onion_reachable": true,
  "consecutive_failures": 0,
  "total_pings": 100,
  "total_failures": 2
}
```

### POST `/faucet`

Claim testnet tokens (disabled on mainnet).

**Request:**
```json
{ "address": "LOSX7dStdPkS9U4MFCmDQfpmvrbMa5WAZfQX1" }
```

---

## gRPC API

Protocol definition: [`los.proto`](../los.proto)

| RPC Method | Description |
|---|---|
| `GetBalance` | Account balance |
| `GetAccount` | Full account details |
| `GetBlock` | Block by hash |
| `GetLatestBlock` | Latest block |
| `SendTransaction` | Submit signed transaction |
| `GetNodeInfo` | Node information |
| `GetValidators` | Validator list |
| `GetBlockHeight` | Current block height |

**gRPC port:** Always REST port + 20,000 (default: `23030`).

---

## USP-01 Token Endpoints

The **USP-01 Token Standard** is deployed as WASM contracts on the UVM. These operations go through the generic `/deploy-contract` and `/call-contract` endpoints, but with specific function signatures documented here.

### Deploy a USP-01 Token

Use `POST /deploy-contract` with a compiled USP-01 WASM binary, then call `init`.

**Init Call:**
```json
{
  "contract_id": "LOSConXXXX...",
  "function": "init",
  "args": ["My Token", "MTK", "11", "1000000", "0", "", "0", ""],
  "caller": "LOSX7dSt...",
  "signature": "hex...",
  "public_key": "hex..."
}
```

| Arg | Field | Type | Description |
|---|---|---|---|
| 0 | `name` | String | Token name (1-64 chars) |
| 1 | `symbol` | String | Token symbol (1-8 chars) |
| 2 | `decimals` | u8 | Decimal places (0-18) |
| 3 | `total_supply` | u128 string | Initial supply assigned to deployer |
| 4 | `is_wrapped` | "0"/"1" | Whether this is a wrapped asset |
| 5 | `wrapped_origin` | String | Source chain identifier (e.g. "ETH") |
| 6 | `max_supply` | u128 string | Max supply cap ("0" = no cap) |
| 7 | `bridge_operator` | address | Address authorized for wrap_mint |

### `transfer`

Transfer tokens from caller to recipient.

```json
{ "function": "transfer", "args": ["LOSRecipient...", "1000"] }
```

### `approve`

Set spending allowance for a spender. Set amount to "0" to revoke.

```json
{ "function": "approve", "args": ["LOSSpender...", "5000"] }
```

### `transfer_from`

Transfer tokens using a pre-approved allowance.

```json
{ "function": "transfer_from", "args": ["LOSOwner...", "LOSRecipient...", "1000"] }
```

### `burn`

Permanently destroy tokens from caller's balance. Reduces total supply.

```json
{ "function": "burn", "args": ["500"] }
```

### `balance_of` (Read-only)

```json
{ "function": "balance_of", "args": ["LOSHolder..."] }
```

**Response:**
```json
{ "account": "LOSHolder...", "balance": "1000" }
```

### `allowance_of` (Read-only)

```json
{ "function": "allowance_of", "args": ["LOSOwner...", "LOSSpender..."] }
```

**Response:**
```json
{ "owner": "LOSOwner...", "spender": "LOSSpender...", "allowance": "5000" }
```

### `total_supply` (Read-only)

```json
{ "function": "total_supply", "args": [] }
```

**Response:**
```json
{ "total_supply": "1000000" }
```

### `token_info` (Read-only)

Returns full token metadata.

**Response:**
```json
{
  "name": "My Token",
  "symbol": "MTK",
  "decimals": 11,
  "total_supply": "1000000",
  "is_wrapped": false,
  "wrapped_origin": "",
  "max_supply": "0",
  "bridge_operator": "",
  "owner": "LOSX7dSt...",
  "contract": "LOSConXXXX...",
  "standard": "USP-01"
}
```

### `wrap_mint` (Bridge Operator Only)

Mint wrapped tokens upon cross-chain deposit verification.

```json
{ "function": "wrap_mint", "args": ["LOSRecipient...", "1000", "0xTxProof..."] }
```

### `wrap_burn`

Burn wrapped tokens for redemption on the source chain.

```json
{ "function": "wrap_burn", "args": ["500", "0xDestinationAddress..."] }
```

**Events emitted:** `USP01:Init`, `USP01:Transfer`, `USP01:Approval`, `USP01:Burn`, `USP01:WrapMint`, `USP01:WrapBurn`.

---

## DEX AMM Endpoints

The **DEX AMM** is a constant-product (x·y=k) automated market maker deployed as a WASM contract. All operations go through `/deploy-contract` and `/call-contract`.

**Constants:** 0.3% fee (30 bps), minimum liquidity 1,000, max fee 1,000 bps.

### `init`

Initialize the DEX contract.

```json
{ "function": "init", "args": [] }
```

### `create_pool`

Create a new liquidity pool with initial reserves.

```json
{
  "function": "create_pool",
  "args": ["LOSConTokenA...", "LOSConTokenB...", "1000000", "500000", "30"]
}
```

| Arg | Field | Type | Description |
|---|---|---|---|
| 0 | `token_a` | address/"LOS" | First token (use "LOS" for native) |
| 1 | `token_b` | address/"LOS" | Second token |
| 2 | `amount_a` | u128 string | Initial reserve for token A |
| 3 | `amount_b` | u128 string | Initial reserve for token B |
| 4 | `fee_bps` | u128 string | Fee in basis points (optional, default 30) |

**LP minted:** `isqrt(amount_a × amount_b) - 1000` (minimum liquidity locked).

### `add_liquidity`

Add proportional liquidity to an existing pool.

```json
{ "function": "add_liquidity", "args": ["0", "100000", "50000", "900"] }
```

| Arg | Field | Description |
|---|---|---|
| 0 | `pool_id` | Pool identifier |
| 1 | `amount_a` | Token A deposit |
| 2 | `amount_b` | Token B deposit |
| 3 | `min_lp_tokens` | Slippage protection: minimum LP tokens to accept |

### `remove_liquidity`

Withdraw proportional reserves by burning LP tokens.

```json
{ "function": "remove_liquidity", "args": ["0", "500", "40000", "20000"] }
```

| Arg | Field | Description |
|---|---|---|
| 0 | `pool_id` | Pool identifier |
| 1 | `lp_amount` | LP tokens to burn |
| 2 | `min_amount_a` | Slippage protection: minimum token A out |
| 3 | `min_amount_b` | Slippage protection: minimum token B out |

### `swap`

Execute a token swap with MEV protection.

```json
{ "function": "swap", "args": ["0", "LOSConTokenA...", "10000", "4800", "1771280000"] }
```

| Arg | Field | Description |
|---|---|---|
| 0 | `pool_id` | Pool identifier |
| 1 | `token_in` | Address of token being sold |
| 2 | `amount_in` | Amount to swap |
| 3 | `min_amount_out` | Slippage protection: minimum output |
| 4 | `deadline` | Unix timestamp deadline (MEV protection) |

**Formula:** `amount_out = (amount_after_fee × reserve_out) / (reserve_in + amount_after_fee)`

### `get_pool` (Read-only)

```json
{ "function": "get_pool", "args": ["0"] }
```

**Response:**
```json
{
  "pool_id": "0",
  "token_a": "LOSConTokenA...",
  "token_b": "LOSConTokenB...",
  "reserve_a": "1000000",
  "reserve_b": "500000",
  "total_lp": "706106",
  "fee_bps": "30",
  "creator": "LOSX7dSt...",
  "last_trade": "1771277598",
  "spot_price_scaled": "2000000000000"
}
```

### `quote` (Read-only)

Get expected swap output without executing.

```json
{ "function": "quote", "args": ["0", "LOSConTokenA...", "10000"] }
```

**Response:**
```json
{
  "amount_out": "4950",
  "fee": "30",
  "price_impact_bps": "100",
  "spot_price_scaled": "2000000000000"
}
```

### `get_position` (Read-only)

Get caller's LP position in a pool.

```json
{ "function": "get_position", "args": ["0"] }
```

**Response:**
```json
{
  "lp_shares": "10000",
  "total_lp": "706106",
  "amount_a": "14158",
  "amount_b": "7079",
  "share_pct_bps": "141"
}
```

### `list_pools` (Read-only)

List all pools in the DEX.

```json
{ "function": "list_pools", "args": [] }
```

**Events emitted:** `DexInit`, `PoolCreated`, `LiquidityAdded`, `LiquidityRemoved`, `Swap`.

---

## CLI Reference

The `los-cli` binary provides command-line access to all node functionality.

**Global flags:** `--rpc <URL>` (default: `http://localhost:3030`), `--config-dir <DIR>` (default: `~/.los`)

### `los-cli wallet` — Wallet Management

| Command | Description |
|---|---|
| `wallet new --name <NAME>` | Create new Dilithium5 wallet |
| `wallet list` | List all wallets |
| `wallet balance <ADDRESS>` | Show wallet balance |
| `wallet export <NAME> --output <PATH>` | Export encrypted wallet |
| `wallet import <PATH> --name <NAME>` | Import wallet |

### `los-cli tx` — Transaction Operations

| Command | Description |
|---|---|
| `tx send --to <ADDR> --amount <LOS> --from <WALLET>` | Send LOS to address |
| `tx status <HASH>` | Query transaction status |

### `los-cli query` — Blockchain Queries

| Command | Description |
|---|---|
| `query block <HEIGHT>` | Get block by height |
| `query account <ADDRESS>` | Get account state |
| `query info` | Network information |
| `query validators` | Get validator set |

### `los-cli validator` — Validator Operations

| Command | Description |
|---|---|
| `validator stake --amount <LOS> --wallet <NAME>` | Stake tokens (min 1,000 LOS) |
| `validator unstake --wallet <NAME>` | Unstake tokens |
| `validator status <ADDRESS>` | Show validator status |
| `validator list` | List active validators |

### `los-cli token` — USP-01 Token Operations

| Command | Description |
|---|---|
| `token deploy --wallet <W> --wasm <PATH> --name <N> --symbol <S> --decimals <D> --total-supply <AMT>` | Deploy USP-01 token |
| `token list` | List all deployed tokens |
| `token info <ADDRESS>` | Show token metadata |
| `token balance --token <ADDR> --holder <ADDR>` | Query token balance |
| `token allowance --token <T> --owner <O> --spender <S>` | Query allowance |
| `token transfer --wallet <W> --token <T> --to <ADDR> --amount <AMT>` | Transfer tokens |
| `token approve --wallet <W> --token <T> --spender <ADDR> --amount <AMT>` | Approve spender |
| `token burn --wallet <W> --token <T> --amount <AMT>` | Burn tokens |
| `token mint --wallet <W> --token <T> --to <ADDR> --amount <AMT>` | Distribute tokens (owner transfer) |

### `los-cli dex` — DEX Operations

| Command | Description |
|---|---|
| `dex deploy --wallet <W> --wasm <PATH>` | Deploy DEX AMM contract |
| `dex pools` | List all DEX pools |
| `dex pool --contract <C> --pool-id <ID>` | Show pool info |
| `dex quote --contract <C> --pool-id <ID> --token-in <T> --amount-in <AMT>` | Get swap quote |
| `dex position --contract <C> --pool-id <ID> --user <ADDR>` | Get LP position |
| `dex create-pool --wallet <W> --contract <C> --token-a <A> --token-b <B> --amount-a <A> --amount-b <B>` | Create liquidity pool |
| `dex add-liquidity --wallet <W> --contract <C> --pool-id <ID> --amount-a <A> --amount-b <B> --min-lp <MIN>` | Add liquidity |
| `dex remove-liquidity --wallet <W> --contract <C> --pool-id <ID> --lp-amount <LP> --min-a <A> --min-b <B>` | Remove liquidity |
| `dex swap --wallet <W> --contract <C> --pool-id <ID> --token-in <T> --amount-in <A> --min-out <MIN>` | Execute swap |

---

## Rate Limits

| Endpoint | Limit |
|---|---|
| `/faucet` | 1 per address per 24 hours |
| `/send` | Anti-spam throttle per address |
| `/burn` | 1 per TXID (globally deduplicated) |
| All endpoints | Per-IP rate limiting |
