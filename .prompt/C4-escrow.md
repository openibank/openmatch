# C4 -- openmatch-escrow

## Purpose

The `openmatch-escrow` crate implements the escrow-first model that is the
foundation of OpeniMatch's order safety guarantees. **No order may enter the
order book without a cryptographically signed freeze proof** attesting that the
required funds have been escrowed on the issuing node.

This crate bridges the gap between `openmatch-types` (which defines the
`FreezeProof` data structure) and `openmatch-core` (which provides
`BalanceManager` for balance accounting). It adds:

1. Ed25519 signing and verification of freeze proofs.
2. Nonce tracking to prevent replay attacks.
3. Expiry enforcement.
4. A high-level `process_order()` pipeline that validates, freezes, signs, and
   enriches an incoming order in one call.

## Crate Metadata

```toml
[package]
name = "openmatch-escrow"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Escrow manager: freeze proof signing, verification, and order enrichment"
license.workspace = true
repository.workspace = true

[features]
default = []
test-helpers = ["openmatch-types/test-helpers"]

[dependencies]
openmatch-types.workspace = true
openmatch-core.workspace = true
ed25519-dalek.workspace = true
chrono.workspace = true
rust_decimal.workspace = true
tracing.workspace = true
rand.workspace = true

[dev-dependencies]
serde_json.workspace = true
openmatch-types = { workspace = true, features = ["test-helpers"] }

[lints]
workspace = true
```

After creating this crate, uncomment the relevant lines in the root
`Cargo.toml`:

```toml
# workspace members
"crates/openmatch-escrow",

# workspace.dependencies
openmatch-escrow = { path = "crates/openmatch-escrow" }
```

## File Layout

```
crates/openmatch-escrow/
  Cargo.toml
  src/
    lib.rs           -- module declarations + re-exports
    escrow_manager.rs -- EscrowManager struct (core logic)
  tests/
    integration_escrow.rs
```

## Existing Types You Depend On

These are already defined in `openmatch-types` -- **do not redefine them**:

```rust
// openmatch_types::freeze
pub struct FreezeProof {
    pub order_id: OrderId,
    pub user_id: UserId,
    pub asset: String,
    pub amount: Decimal,
    pub issuer_node: NodeId,
    pub signature: Vec<u8>,
    pub nonce: u64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl FreezeProof {
    /// Canonical signing payload:
    /// `order_id(16) || user_id(16) || asset(utf8) || amount(str) || nonce(8)`
    pub fn signing_payload(&self) -> Vec<u8>;
    pub fn is_expired(&self) -> bool;
    pub fn expires_within(&self, duration: chrono::Duration) -> bool;
    pub fn time_until_expiry(&self) -> chrono::Duration;
}
```

```rust
// openmatch_types::error (relevant variants)
pub enum OpenmatchError {
    InvalidFreezeProof { reason: String },   // OM_ERR_300
    FreezeProofExpired,                       // OM_ERR_301
    FreezeSignatureInvalid,                   // OM_ERR_302
    FreezeNonceReused,                        // OM_ERR_303
    InsufficientBalance { needed, available },// OM_ERR_200
    InvalidOrder { reason: String },          // OM_ERR_101
    // ...
}
```

```rust
// openmatch_core::BalanceManager (relevant methods)
impl BalanceManager {
    pub fn get(&self, user_id: &UserId, asset: &str) -> BalanceEntry;
    pub fn freeze(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()>;
    pub fn unfreeze(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()>;
}
```

```rust
// openmatch_types::order
pub struct Order {
    pub id: OrderId,
    pub user_id: UserId,
    pub market: MarketPair,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub status: OrderStatus,       // starts as PendingFreeze
    pub price: Option<Decimal>,
    pub quantity: Decimal,
    pub remaining_qty: Decimal,
    pub freeze_proof: FreezeProof,
    pub batch_id: Option<BatchId>,
    pub origin_node: NodeId,
    pub sequence: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum OrderStatus {
    PendingFreeze, Active, PartiallyFilled, Filled, Cancelled, Rejected, Expired,
}
```

## Architecture

```
         +-------------------------------------------------+
         |              EscrowManager                      |
         |                                                 |
         |  signing_key: ed25519_dalek::SigningKey          |
         |  verifying_key: ed25519_dalek::VerifyingKey      |
         |  node_id: NodeId                                |
         |  used_nonces: HashSet<u64>                      |
         |  proof_ttl: chrono::Duration                    |
         |                                                 |
         |  create_freeze_proof()                          |
         |  verify_freeze_proof()                          |
         |  process_order()                                |
         +-------------------------------------------------+
                    |                      |
                    v                      v
          BalanceManager           FreezeProof (types)
          (openmatch-core)         (openmatch-types)
```

## Implementation Details

### `src/lib.rs`

```rust
//! # openmatch-escrow
//!
//! Escrow manager for the OpeniMatch matching engine.
//!
//! This crate implements the escrow-first model: every order must have a
//! cryptographically signed [`FreezeProof`] before entering the order book.
//! The [`EscrowManager`] handles:
//!
//! - Creating and signing freeze proofs with ed25519
//! - Verifying freeze proof signatures, expiry, and nonce uniqueness
//! - Integrating with [`BalanceManager`](openmatch_core::BalanceManager) to
//!   freeze/unfreeze user balances
//! - Processing incoming orders end-to-end via [`EscrowManager::process_order()`]

pub mod escrow_manager;

pub use escrow_manager::EscrowManager;
```

### `src/escrow_manager.rs` -- Struct Definition

```rust
use std::collections::HashSet;

use chrono::{Duration, Utc};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use openmatch_core::BalanceManager;
use openmatch_types::*;
use rust_decimal::Decimal;

/// Default freeze proof time-to-live: 5 minutes.
const DEFAULT_PROOF_TTL_SECS: i64 = 300;

/// Manages the freeze/unfreeze lifecycle for escrow-first order handling.
///
/// # Responsibilities
///
/// 1. **Sign** freeze proofs using the node's ed25519 private key.
/// 2. **Verify** incoming proofs: check signature, expiry, and nonce uniqueness.
/// 3. **Track nonces** in a `HashSet<u64>` to prevent replay attacks.
/// 4. **Integrate with `BalanceManager`** to freeze/unfreeze user balances.
/// 5. **Process orders** end-to-end: validate -> check balance -> freeze -> sign -> enrich.
pub struct EscrowManager {
    /// Ed25519 signing key for this node.
    signing_key: SigningKey,
    /// Corresponding verification key (derived from `signing_key`).
    verifying_key: VerifyingKey,
    /// This node's identity.
    node_id: NodeId,
    /// Set of already-used nonces (prevents replay).
    used_nonces: HashSet<u64>,
    /// Time-to-live for newly created freeze proofs.
    proof_ttl: Duration,
    /// Monotonic nonce counter for proofs this node creates.
    next_nonce: u64,
}
```

### `EscrowManager::new()`

```rust
impl EscrowManager {
    /// Create a new `EscrowManager` from an ed25519 signing key.
    ///
    /// The `NodeId` is derived from the public key bytes of the signing key.
    pub fn new(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        let node_id = NodeId::from_pubkey(verifying_key.to_bytes());
        Self {
            signing_key,
            verifying_key,
            node_id,
            used_nonces: HashSet::new(),
            proof_ttl: Duration::seconds(DEFAULT_PROOF_TTL_SECS),
            next_nonce: 0,
        }
    }

    /// Create with a custom proof TTL.
    pub fn with_proof_ttl(signing_key: SigningKey, ttl: Duration) -> Self {
        let mut mgr = Self::new(signing_key);
        mgr.proof_ttl = ttl;
        mgr
    }

    /// Return this node's identity.
    pub fn node_id(&self) -> NodeId { self.node_id }

    /// Return a reference to the verifying (public) key.
    pub fn verifying_key(&self) -> &VerifyingKey { &self.verifying_key }
}
```

### `create_freeze_proof()`

Creates a signed freeze proof for a given order.

```rust
impl EscrowManager {
    /// Create a new signed freeze proof.
    ///
    /// # Arguments
    /// - `order_id`: the order this proof covers
    /// - `user_id`: the user whose balance is frozen
    /// - `asset`:   the asset being frozen (e.g., "USDT" for a buy)
    /// - `amount`:  the amount to freeze
    ///
    /// # Returns
    /// A `FreezeProof` with a valid ed25519 signature, unique nonce,
    /// and expiry set to `now + proof_ttl`.
    pub fn create_freeze_proof(
        &mut self,
        order_id: OrderId,
        user_id: UserId,
        asset: &str,
        amount: Decimal,
    ) -> FreezeProof {
        let nonce = self.next_nonce;
        self.next_nonce += 1;
        self.used_nonces.insert(nonce);

        let now = Utc::now();
        let mut proof = FreezeProof {
            order_id,
            user_id,
            asset: asset.to_string(),
            amount,
            issuer_node: self.node_id,
            signature: Vec::new(), // placeholder, filled below
            nonce,
            created_at: now,
            expires_at: now + self.proof_ttl,
        };

        // Sign the canonical payload
        let payload = proof.signing_payload();
        let signature = self.signing_key.sign(&payload);
        proof.signature = signature.to_bytes().to_vec();

        tracing::debug!(
            order = %order_id,
            user = %user_id,
            asset,
            amount = %amount,
            nonce,
            "Freeze proof created"
        );

        proof
    }
}
```

### `verify_freeze_proof()`

Verifies an incoming freeze proof from any node.

```rust
impl EscrowManager {
    /// Verify a freeze proof.
    ///
    /// Checks:
    /// 1. Signature validity against `expected_pubkey`
    /// 2. Proof has not expired
    /// 3. Nonce has not been seen before (replay prevention)
    ///
    /// On success the nonce is recorded in `used_nonces`.
    ///
    /// # Arguments
    /// - `proof`: the freeze proof to verify
    /// - `expected_pubkey`: the ed25519 public key of the issuing node
    ///
    /// # Errors
    /// - `FreezeSignatureInvalid` if signature doesn't verify
    /// - `FreezeProofExpired` if `proof.is_expired()`
    /// - `FreezeNonceReused` if nonce was already seen
    pub fn verify_freeze_proof(
        &mut self,
        proof: &FreezeProof,
        expected_pubkey: &VerifyingKey,
    ) -> Result<()> {
        // 1. Check expiry first (cheapest check)
        if proof.is_expired() {
            return Err(OpenmatchError::FreezeProofExpired);
        }

        // 2. Check nonce uniqueness
        if self.used_nonces.contains(&proof.nonce) {
            return Err(OpenmatchError::FreezeNonceReused);
        }

        // 3. Verify ed25519 signature
        let payload = proof.signing_payload();
        let sig_bytes: [u8; 64] = proof
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| OpenmatchError::FreezeSignatureInvalid)?;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        expected_pubkey
            .verify(&payload, &signature)
            .map_err(|_| OpenmatchError::FreezeSignatureInvalid)?;

        // Record the nonce
        self.used_nonces.insert(proof.nonce);

        tracing::debug!(
            order = %proof.order_id,
            nonce = proof.nonce,
            issuer = %proof.issuer_node,
            "Freeze proof verified"
        );

        Ok(())
    }
}
```

### `process_order()`

The end-to-end pipeline for accepting an incoming order.

```rust
impl EscrowManager {
    /// Process an incoming order end-to-end.
    ///
    /// Pipeline:
    /// 1. **Validate** the order (price > 0 for limit, qty > 0, etc.)
    /// 2. **Determine freeze asset and amount**:
    ///    - Buy orders freeze the quote asset (`price * quantity`)
    ///    - Sell orders freeze the base asset (`quantity`)
    /// 3. **Check available balance** via `BalanceManager`
    /// 4. **Freeze** the balance via `BalanceManager::freeze()`
    /// 5. **Create a signed freeze proof** via `create_freeze_proof()`
    /// 6. **Enrich the order**: attach the proof, set status to `Active`,
    ///    set `origin_node`
    ///
    /// # Arguments
    /// - `order`: the order to process (status should be `PendingFreeze`)
    /// - `balance_mgr`: mutable reference to the node's balance manager
    ///
    /// # Returns
    /// The enriched `Order` with a valid `FreezeProof` and `Active` status.
    ///
    /// # Errors
    /// - `InvalidOrder` if validation fails
    /// - `InsufficientBalance` if the user cannot cover the required freeze
    /// - Any error from `BalanceManager::freeze()`
    pub fn process_order(
        &mut self,
        mut order: Order,
        balance_mgr: &mut BalanceManager,
    ) -> Result<Order> {
        // 1. Validate
        self.validate_order(&order)?;

        // 2. Determine freeze asset and amount
        let (freeze_asset, freeze_amount) = self.compute_freeze_requirements(&order)?;

        // 3 + 4. Check balance and freeze (BalanceManager::freeze checks availability)
        balance_mgr.freeze(&order.user_id, &freeze_asset, freeze_amount)?;

        // 5. Create signed proof
        let proof = self.create_freeze_proof(
            order.id,
            order.user_id,
            &freeze_asset,
            freeze_amount,
        );

        // 6. Enrich the order
        order.freeze_proof = proof;
        order.status = OrderStatus::Active;
        order.origin_node = self.node_id;
        order.updated_at = Utc::now();

        tracing::info!(
            order = %order.id,
            user = %order.user_id,
            side = %order.side,
            freeze_asset = %freeze_asset,
            freeze_amount = %freeze_amount,
            "Order processed and escrow frozen"
        );

        Ok(order)
    }

    /// Validate basic order invariants.
    fn validate_order(&self, order: &Order) -> Result<()> {
        if order.quantity <= Decimal::ZERO {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Order quantity must be positive".into(),
            });
        }
        if order.order_type == OrderType::Limit {
            match order.price {
                None => return Err(OpenmatchError::InvalidOrder {
                    reason: "Limit order must have a price".into(),
                }),
                Some(p) if p <= Decimal::ZERO => return Err(OpenmatchError::InvalidOrder {
                    reason: "Limit order price must be positive".into(),
                }),
                _ => {}
            }
        }
        Ok(())
    }

    /// Compute what asset and how much to freeze for an order.
    ///
    /// - **Buy**: freeze `price * quantity` of the quote asset
    /// - **Sell**: freeze `quantity` of the base asset
    /// - **Market Buy**: return an error (market buys need a maximum quote
    ///   amount specified externally; this is a simplification)
    fn compute_freeze_requirements(&self, order: &Order) -> Result<(String, Decimal)> {
        match (order.side, order.order_type) {
            (OrderSide::Buy, OrderType::Limit) => {
                let price = order.price.ok_or_else(|| OpenmatchError::InvalidOrder {
                    reason: "Buy limit order must have price".into(),
                })?;
                let quote_amount = price * order.quantity;
                Ok((order.market.quote.clone(), quote_amount))
            }
            (OrderSide::Sell, _) => {
                Ok((order.market.base.clone(), order.quantity))
            }
            (OrderSide::Buy, OrderType::Market) => {
                // Market buys require a quote ceiling; for now we reject
                // them at the escrow level. The API layer should convert
                // market buys into limit orders with a slippage cap.
                Err(OpenmatchError::InvalidOrder {
                    reason: "Market buy orders must be converted to limit with max price before escrow".into(),
                })
            }
            (_, OrderType::Cancel) => {
                Err(OpenmatchError::InvalidOrder {
                    reason: "Cancel orders do not require escrow".into(),
                })
            }
        }
    }
}
```

### Additional Utility Methods

```rust
impl EscrowManager {
    /// Release a freeze -- call when an order is cancelled or expires.
    ///
    /// Unfreezes the amount on the `BalanceManager` that was originally
    /// frozen for this proof.
    pub fn release_freeze(
        &self,
        proof: &FreezeProof,
        balance_mgr: &mut BalanceManager,
    ) -> Result<()> {
        balance_mgr.unfreeze(&proof.user_id, &proof.asset, proof.amount)
    }

    /// Number of nonces tracked (useful for monitoring / tests).
    pub fn nonce_count(&self) -> usize {
        self.used_nonces.len()
    }

    /// Reset nonce tracking (use with caution; primarily for testing).
    pub fn clear_nonces(&mut self) {
        self.used_nonces.clear();
        self.next_nonce = 0;
    }
}
```

## Conventions and Style

Follow the patterns established in the existing crates:

- **Rust edition 2024**, MSRV 1.85.
- `unsafe_code = "forbid"` -- no unsafe.
- Clippy `pedantic` + `all` at warn level. Allowed lints: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`.
- `rustfmt.toml`: `max_width = 100`, `imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`, `use_field_init_shorthand = true`.
- Use `openmatch_types::Result<T>` (alias for `std::result::Result<T, OpenmatchError>`).
- Use `tracing::debug!` / `tracing::info!` for structured logging, not `println!`.
- Use `#[must_use]` on pure query methods.
- Doc comments (`///`) on every public item.
- Module-level doc comments (`//!`) at the top of each file.

## Error Handling

Use the existing `OpenmatchError` variants -- **do not add new variants** unless
absolutely necessary. The relevant variants for this crate:

| Situation | Error Variant |
|---|---|
| Order fails validation | `InvalidOrder { reason }` |
| Not enough available balance | `InsufficientBalance { needed, available }` |
| Not enough frozen balance (unfreeze) | `InsufficientFrozen` |
| Proof structurally bad | `InvalidFreezeProof { reason }` |
| Proof expired | `FreezeProofExpired` |
| Ed25519 sig fails | `FreezeSignatureInvalid` |
| Nonce reuse | `FreezeNonceReused` |

## Test Strategy

### Unit Tests (in `src/escrow_manager.rs`)

Write `#[cfg(test)] mod tests` at the bottom of `escrow_manager.rs`:

1. **`create_and_verify_roundtrip`** -- Create a proof, then verify it with the
   same node's public key. Must succeed.

2. **`verify_rejects_expired_proof`** -- Create a proof with TTL of 0 seconds
   (or set `expires_at` to the past). `verify_freeze_proof()` must return
   `FreezeProofExpired`.

3. **`verify_rejects_reused_nonce`** -- Create a proof, verify it (nonce
   recorded), then attempt to verify the same proof again. Must return
   `FreezeNonceReused`.

4. **`verify_rejects_wrong_key`** -- Create a proof signed by key A, verify
   with key B's public key. Must return `FreezeSignatureInvalid`.

5. **`verify_rejects_tampered_payload`** -- Create a proof, modify the amount,
   then verify. Must return `FreezeSignatureInvalid`.

6. **`process_order_buy_limit`** -- Create a buy limit order, process it. Check
   that:
   - `order.status == Active`
   - `order.freeze_proof.amount == price * quantity`
   - `order.freeze_proof.asset == market.quote`
   - `balance_mgr` has the expected frozen amount
   - The freeze proof signature verifies

7. **`process_order_sell_limit`** -- Same as above but for a sell order.
   Freeze asset should be the base asset, amount should be the quantity.

8. **`process_order_insufficient_balance`** -- Attempt to process an order when
   the user has insufficient available balance. Must return
   `InsufficientBalance`.

9. **`process_order_invalid_qty`** -- Order with `quantity = 0`. Must return
   `InvalidOrder`.

10. **`release_freeze_returns_to_available`** -- Process an order, then call
    `release_freeze()`. Verify the balance returns from frozen to available.

### Test Helpers

Use `ed25519_dalek::SigningKey::generate()` with `rand::rngs::OsRng` (or
`rand::thread_rng()`) to create test signing keys:

```rust
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;

fn test_signing_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}
```

Use `FreezeProof::dummy()` from `openmatch-types` (with `test-helpers` feature)
only for tests that don't need real signatures. For escrow-specific tests, use
the `EscrowManager` to produce real proofs.

### Integration Test (`tests/integration_escrow.rs`)

Test the full pipeline: deposit -> process_order -> verify proof -> settle:

```rust
//! Integration test: escrow pipeline
//!
//! deposit -> process_order -> verify_freeze_proof -> (matching would happen
//! here) -> release_freeze on cancel

#[test]
fn escrow_full_pipeline() {
    // 1. Create EscrowManager with a fresh key
    // 2. Create BalanceManager, deposit funds for a user
    // 3. Build an Order with status = PendingFreeze
    // 4. Call process_order() -- should succeed, order becomes Active
    // 5. Verify the freeze proof on the enriched order
    // 6. Check balance: available decreased, frozen increased
    // 7. Call release_freeze() (simulating cancel)
    // 8. Check balance: frozen back to 0, available restored
}

#[test]
fn escrow_rejects_double_freeze_via_nonce() {
    // Process two orders -- each gets a unique nonce
    // Manually craft a proof with the first nonce and try to verify it
    // as if it were a new proof -- should fail with FreezeNonceReused
}
```

## Key Invariants

1. **Every freeze proof has a unique nonce.** The `next_nonce` counter is
   monotonic and the nonce is inserted into `used_nonces` immediately on
   creation. Verification also inserts into `used_nonces` on success.

2. **Signature covers the canonical payload.** The signing payload is
   `order_id(16) || user_id(16) || asset(utf8) || amount(str) || nonce(8)` as
   defined by `FreezeProof::signing_payload()`. Do not modify this format.

3. **Expiry is checked before signature verification.** This avoids wasting CPU
   on signature verification for stale proofs.

4. **`process_order()` is atomic w.r.t. balance.** If any step fails after
   `BalanceManager::freeze()`, the caller must unfreeze. In the current
   design, the proof creation step after freeze cannot fail (signing is
   infallible), so this is not a concern yet. However, if you add fallible
   steps after freeze, wrap the whole thing in a compensating transaction or
   unfreeze on error.

5. **`EscrowManager` is single-threaded.** It holds mutable state
   (`used_nonces`, `next_nonce`). If concurrency is needed in the future,
   wrap in `Mutex` or `RwLock` at the caller level. Do not add synchronization
   primitives inside this crate yet.

## What NOT to Do

- Do not add a database or persistence layer. Nonce tracking is in-memory.
  Persistence will be handled by `openmatch-persistence` later.
- Do not add network calls. Cross-node proof verification happens when another
  crate calls `verify_freeze_proof()` with the remote node's public key.
- Do not modify `FreezeProof` fields or `signing_payload()` in
  `openmatch-types`. Those are stable.
- Do not add new error variants to `OpenmatchError` unless the existing ones
  are truly insufficient. Prefer `InvalidFreezeProof { reason: "..." }` for
  novel validation failures.
- Do not use `tokio` or async. This crate is purely synchronous.
