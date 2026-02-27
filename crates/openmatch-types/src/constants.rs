//! System-wide constants for the OpenMatch matching engine.

/// Maximum decimal precision for prices (8 decimal places).
pub const PRICE_PRECISION: u32 = 8;

/// Maximum decimal precision for quantities (8 decimal places).
pub const QTY_PRECISION: u32 = 8;

/// Default epoch COLLECT phase duration in milliseconds.
pub const DEFAULT_COLLECT_MS: u64 = 1000;

/// Default SEAL phase duration in milliseconds.
pub const DEFAULT_SEAL_MS: u64 = 200;

/// Default MATCH phase timeout in milliseconds.
pub const DEFAULT_MATCH_TIMEOUT_MS: u64 = 500;

/// Default FINALIZE phase timeout in milliseconds.
pub const DEFAULT_FINALIZE_TIMEOUT_MS: u64 = 2000;

/// Alias: default SETTLE phase timeout in milliseconds (same as FINALIZE).
pub const DEFAULT_SETTLE_TIMEOUT_MS: u64 = DEFAULT_FINALIZE_TIMEOUT_MS;

/// Default seal grace period in milliseconds.
pub const DEFAULT_SEAL_GRACE_MS: u64 = 50;

/// Maximum orders allowed in a single batch.
pub const MAX_ORDERS_PER_BATCH: usize = 100_000;

/// Maximum open orders per user per market (default).
pub const DEFAULT_MAX_ORDERS_PER_USER: usize = 200;

/// Maximum peers in P2P gossip network.
pub const DEFAULT_MAX_PEERS: usize = 50;

/// Default gossip port.
pub const DEFAULT_GOSSIP_PORT: u16 = 9944;

/// Default API listen port.
pub const DEFAULT_API_PORT: u16 = 8080;

/// Maximum orders per user in a single epoch (anti-flood).
pub const MAX_ORDERS_PER_USER_PER_EPOCH: usize = 50;

/// Rate limit window for order submission (milliseconds).
pub const ORDER_RATE_LIMIT_WINDOW_MS: u64 = 1000;

/// Maximum orders per user within the rate limit window.
pub const ORDER_RATE_LIMIT_COUNT: usize = 20;

/// Maximum price deviation multiplier from the last known price
/// before flagging as suspicious (e.g., 10 = 10x deviation).
pub const MAX_PRICE_DEVIATION_MULTIPLIER: u64 = 10;

/// Maximum nonce entries to retain per node before pruning oldest.
pub const MAX_NONCE_ENTRIES_PER_NODE: usize = 100_000;

/// Settlement idempotency cache size (number of trade IDs to remember).
pub const SETTLEMENT_IDEMPOTENCY_CACHE_SIZE: usize = 500_000;

/// Version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Engine name.
pub const ENGINE_NAME: &str = "OpenMatch";
