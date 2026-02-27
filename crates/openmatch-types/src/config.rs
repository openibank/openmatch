//! Configuration types for OpenMatch nodes and markets.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::{constants, EpochConfig, NodeId};

/// Configuration for a single OpenMatch node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// This node's identity (ed25519 public key).
    pub node_id: NodeId,
    /// Address to listen on for the REST/WS API.
    pub listen_addr: SocketAddr,
    /// Path to the data directory (WAL, snapshots, etc.).
    pub data_dir: String,
    /// Epoch timing configuration.
    pub epoch: EpochConfig,
    /// P2P network configuration.
    pub network: NetworkConfig,
    /// Markets supported by this node.
    pub markets: Vec<MarketConfig>,
}

/// P2P network configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Bootstrap peer addresses (multiaddr format).
    pub bootstrap_peers: Vec<String>,
    /// Port for gossip protocol.
    pub gossip_port: u16,
    /// Maximum number of connected peers.
    pub max_peers: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bootstrap_peers: Vec::new(),
            gossip_port: constants::DEFAULT_GOSSIP_PORT,
            max_peers: constants::DEFAULT_MAX_PEERS,
        }
    }
}

/// Per-market configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    /// Base asset (e.g., "BTC").
    pub base: String,
    /// Quote asset (e.g., "USDT").
    pub quote: String,
    /// Minimum order size in base asset.
    pub min_order_size: Decimal,
    /// Tick size (price granularity).
    pub tick_size: Decimal,
    /// Lot size (quantity granularity).
    pub lot_size: Decimal,
    /// Maximum number of open orders per user for this market.
    pub max_orders_per_user: usize,
}

impl MarketConfig {
    /// Create a default BTC/USDT market config.
    #[must_use]
    pub fn btc_usdt() -> Self {
        Self {
            base: "BTC".to_string(),
            quote: "USDT".to_string(),
            min_order_size: Decimal::new(1, 5),   // 0.00001 BTC
            tick_size: Decimal::new(1, 2),         // 0.01 USDT
            lot_size: Decimal::new(1, 5),          // 0.00001 BTC
            max_orders_per_user: constants::DEFAULT_MAX_ORDERS_PER_USER,
        }
    }

    /// Create a default ETH/USDT market config.
    #[must_use]
    pub fn eth_usdt() -> Self {
        Self {
            base: "ETH".to_string(),
            quote: "USDT".to_string(),
            min_order_size: Decimal::new(1, 4),   // 0.0001 ETH
            tick_size: Decimal::new(1, 2),         // 0.01 USDT
            lot_size: Decimal::new(1, 4),          // 0.0001 ETH
            max_orders_per_user: constants::DEFAULT_MAX_ORDERS_PER_USER,
        }
    }

    /// Returns the market symbol (e.g., "BTC/USDT").
    #[must_use]
    pub fn symbol(&self) -> String {
        format!("{}/{}", self.base, self.quote)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_config_btc_usdt() {
        let cfg = MarketConfig::btc_usdt();
        assert_eq!(cfg.symbol(), "BTC/USDT");
        assert!(cfg.min_order_size > Decimal::ZERO);
        assert!(cfg.tick_size > Decimal::ZERO);
    }

    #[test]
    fn network_config_defaults() {
        let cfg = NetworkConfig::default();
        assert_eq!(cfg.gossip_port, 9944);
        assert_eq!(cfg.max_peers, 50);
        assert!(cfg.bootstrap_peers.is_empty());
    }

    #[test]
    fn market_config_serde_roundtrip() {
        let cfg = MarketConfig::btc_usdt();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: MarketConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg.base, back.base);
        assert_eq!(cfg.quote, back.quote);
        assert_eq!(cfg.tick_size, back.tick_size);
    }
}
