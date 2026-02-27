# B1 — Deployment & Infrastructure

> **Status**: 🔲 TODO
> **Directories**: `docker/`, `benchmarks/`, `.github/`

## Dockerfile (Multi-Stage)

```dockerfile
# Stage 1: Build
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p openmatch-cli

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libpq5 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/openmatch /usr/local/bin/
COPY --from=builder /app/docker/default-config.toml /etc/openmatch/openmatch.toml
EXPOSE 9001 9002 9944 9080
VOLUME /var/lib/openmatch
ENTRYPOINT ["openmatch"]
CMD ["start", "--config", "/etc/openmatch/openmatch.toml"]
```

## docker-compose.yml

```yaml
services:
  openmatch:
    build: .
    ports:
      - "9001:9001"   # REST API
      - "9002:9002"   # WebSocket
      - "9944:9944"   # Gossip
      - "9080:9080"   # Metrics
    volumes:
      - openmatch-data:/var/lib/openmatch
    depends_on:
      postgres: { condition: service_healthy }
    environment:
      - DATABASE_URL=postgresql://openmatch:openmatch@postgres/openmatch

  postgres:
    image: postgres:16
    environment:
      POSTGRES_DB: openmatch
      POSTGRES_USER: openmatch
      POSTGRES_PASSWORD: openmatch
    ports: ["5432:5432"]
    volumes: [postgres-data:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U openmatch"]

  prometheus:
    image: prom/prometheus:latest
    volumes: [./docker/prometheus.yml:/etc/prometheus/prometheus.yml]
    ports: ["9090:9090"]

  grafana:
    image: grafana/grafana:latest
    ports: ["3000:3000"]
    volumes: [./docker/grafana-dashboards:/var/lib/grafana/dashboards]

volumes:
  openmatch-data:
  postgres-data:
```

## Service Ports

| Port | Service |
|------|---------|
| 9001 | REST API |
| 9002 | WebSocket |
| 9944 | P2P Gossip |
| 9080 | Prometheus metrics |
| 5432 | PostgreSQL |
| 3000 | Grafana |
| 9090 | Prometheus |

## GitHub Actions CI

```yaml
name: CI
on: [push, pull_request]
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
        with: { components: "rustfmt,clippy" }
      - run: cargo fmt --check
      - run: cargo clippy --workspace -- -D warnings

  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
      - run: cargo test --workspace

  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
      - run: cargo build --release -p openmatch-cli

  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
      - run: cargo bench --workspace
```

## Benchmarks (`benchmarks/`)

Using `criterion`:

```rust
fn bench_batch_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("matching");
    for size in [1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("batch", size),
            &size,
            |b, &size| b.iter(|| match_random_batch(size)),
        );
    }
    group.finish();
}
```

### Targets

| Metric | Target |
|--------|--------|
| Match 1K orders | <1ms |
| Match 10K orders | <10ms |
| Match 100K orders | <100ms |
| Intra-node settlement | <1ms/trade |
| API response (orderbook) | <5ms p99 |
| WebSocket latency | <10ms p99 |

## Monitoring (Prometheus Metrics)

```
openmatch_epoch_duration_seconds{phase="collect|match|settle"}
openmatch_trades_total{market="BTC/USDT"}
openmatch_orders_total{market="BTC/USDT",side="buy|sell"}
openmatch_matching_latency_seconds
openmatch_settlement_latency_seconds{tier="1|2|3"}
openmatch_orderbook_depth{market="BTC/USDT",side="bid|ask"}
openmatch_connected_peers
```
