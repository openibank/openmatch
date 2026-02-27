# A1 — openmatch-cli

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-cli/` (binary crate)
> **Depends on**: `openmatch-types`, `openmatch-core`, `openmatch-epoch`, `openmatch-api`, `openmatch-persistence`, `clap`, `colored`, `tokio`

## Purpose

Command-line interface for managing OpeniMatch nodes. The `openmatch` binary.

## Commands

```
openmatch init                              Generate keypair + default config
openmatch start [--config openmatch.toml]   Start node
openmatch status                            Query running node health
openmatch keygen                            Generate ed25519 keypair

openmatch deposit <user_id> <asset> <amount>
openmatch withdraw <user_id> <asset> <amount>
openmatch balance <user_id>

openmatch order <market> <side> <price> <qty>
openmatch cancel <order_id>
openmatch book <market>                     Show orderbook (colored table)
openmatch trades <market>                   Show recent trades

openmatch epoch status                      Current epoch phase + timing
openmatch peers                             List connected peers
```

## Config File (`openmatch.toml`)

```toml
[node]
data_dir = "/var/lib/openmatch"
listen_addr = "0.0.0.0:9001"

[epoch]
collect_duration_ms = 1000
match_timeout_ms = 500
settle_timeout_ms = 2000

[network]
gossip_port = 9944
bootstrap_peers = []
max_peers = 50

[[markets]]
base = "BTC"
quote = "USDT"
min_order_size = "0.00001"
tick_size = "0.01"
lot_size = "0.00001"

[[markets]]
base = "ETH"
quote = "USDT"
min_order_size = "0.0001"
tick_size = "0.01"
lot_size = "0.0001"
```

## `init` Command

1. Generate ed25519 keypair → save to `{data_dir}/node.key`
2. Print public key (NodeId) to stdout
3. Generate default `openmatch.toml` in current directory
4. Create data directory structure

## `start` Command

1. Load config from `openmatch.toml`
2. Load or generate node keypair
3. Initialize PostgreSQL connection pool
4. Start epoch controller
5. Start API server (REST + WebSocket)
6. Start gossip network
7. Print startup banner with node info
8. Block on shutdown signal (Ctrl+C)

## Display Formatting

```
┌─────────────────── BTC/USDT Orderbook ───────────────────┐
│  Price        │ Qty          │ Total        │ Side       │
├───────────────┼──────────────┼──────────────┼────────────┤
│  50,100.00    │ 0.50000      │ 0.50000      │ ASK (red)  │
│  50,050.00    │ 1.20000      │ 1.70000      │ ASK (red)  │
│  ────────────── spread: 50.00 ────────────────────────── │
│  50,000.00    │ 2.00000      │ 2.00000      │ BID (green)│
│  49,950.00    │ 0.80000      │ 2.80000      │ BID (green)│
└───────────────┴──────────────┴──────────────┴────────────┘
```

## Testing

1. CLI argument parsing: all commands parse correctly with clap
2. Config file: generate → load → verify defaults
3. Keygen: generate keypair → verify valid ed25519
4. Display formatting: verify table output
