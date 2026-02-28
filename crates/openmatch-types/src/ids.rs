//! Globally unique identifiers used throughout OpenMatch.
//!
//! All entity IDs use UUIDv7 for time-ordered lexicographic sorting,
//! except `NodeId` which uses the ed25519 public key directly.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// OrderId
// ---------------------------------------------------------------------------

/// Globally unique order identifier. Uses UUIDv7 for time-ordered sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct OrderId(pub Uuid);

impl OrderId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// Extract the embedded timestamp (milliseconds since UNIX epoch) from UUIDv7.
    #[must_use]
    pub fn timestamp_ms(&self) -> u64 {
        let bytes = self.0.as_bytes();
        u64::from_be_bytes([
            0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ])
    }
}

impl Default for OrderId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// UserId
// ---------------------------------------------------------------------------

/// Unique identifier for a user / trading account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// Unique identifier for a node in the P2P network.
/// This is the raw ed25519 public key (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    #[must_use]
    pub fn from_pubkey(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node:{}", hex::encode(&self.0[..8]))
    }
}

// ---------------------------------------------------------------------------
// SpendRightId (NEW in v0.2 — the cryptographic pre-commitment ID)
// ---------------------------------------------------------------------------

/// Unique identifier for a SpendRight (escrow reservation token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SpendRightId(pub Uuid);

impl SpendRightId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SpendRightId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SpendRightId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sr:{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// EpochId (NEW in v0.2 — replaces BatchId as primary epoch identifier)
// ---------------------------------------------------------------------------

/// Monotonically increasing identifier for an epoch cycle.
///
/// Each epoch runs: COLLECT → SEAL → MATCH → FINALIZE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct EpochId(pub u64);

impl EpochId {
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for EpochId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch:{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// MarketPair
// ---------------------------------------------------------------------------

/// A trading pair (e.g., BTC/USDT).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct MarketPair {
    pub base: String,
    pub quote: String,
}

impl MarketPair {
    #[must_use]
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            quote: quote.into(),
        }
    }

    #[must_use]
    pub fn symbol(&self) -> String {
        format!("{}/{}", self.base, self.quote)
    }
}

impl fmt::Display for MarketPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.quote)
    }
}

// ---------------------------------------------------------------------------
// TradeId
// ---------------------------------------------------------------------------

/// Globally unique trade identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TradeId(pub Uuid);

impl TradeId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Deterministic `TradeId` from epoch ID and fill sequence.
    ///
    /// Every node generates the **exact same** `TradeId` for the same fill
    /// within the same epoch — critical for cross-node determinism.
    #[must_use]
    pub fn deterministic(epoch_id: u64, fill_sequence: u64) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"openmatch:trade_id:v2:");
        hasher.update(epoch_id.to_le_bytes());
        hasher.update(fill_sequence.to_le_bytes());
        let hash = hasher.finalize();
        let bytes: [u8; 16] = hash[..16].try_into().expect("SHA-256 produces 32 bytes");
        Self(Uuid::from_bytes(bytes))
    }
}

impl Default for TradeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TradeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Legacy alias. Prefer [`EpochId`] in new code.
pub type BatchId = EpochId;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_id_uniqueness() {
        let a = OrderId::new();
        let b = OrderId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn order_id_ordering() {
        let a = OrderId::new();
        let b = OrderId::new();
        assert!(a < b);
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn order_id_timestamp_extraction() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let id = OrderId::new();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let ts = id.timestamp_ms();
        assert!(
            ts >= before && ts <= after,
            "ts={ts}, before={before}, after={after}"
        );
    }

    #[test]
    fn spend_right_id_uniqueness() {
        let a = SpendRightId::new();
        let b = SpendRightId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn epoch_id_next() {
        let e = EpochId(5);
        assert_eq!(e.next(), EpochId(6));
    }

    #[test]
    fn trade_id_deterministic() {
        let a = TradeId::deterministic(100, 0);
        let b = TradeId::deterministic(100, 0);
        assert_eq!(a, b);
        let c = TradeId::deterministic(100, 1);
        assert_ne!(a, c);
    }

    #[test]
    fn market_pair_symbol() {
        let pair = MarketPair::new("BTC", "USDT");
        assert_eq!(pair.symbol(), "BTC/USDT");
    }

    #[test]
    fn serde_roundtrips() {
        let oid = OrderId::new();
        let json = serde_json::to_string(&oid).unwrap();
        let back: OrderId = serde_json::from_str(&json).unwrap();
        assert_eq!(oid, back);

        let srid = SpendRightId::new();
        let json = serde_json::to_string(&srid).unwrap();
        let back: SpendRightId = serde_json::from_str(&json).unwrap();
        assert_eq!(srid, back);
    }
}
