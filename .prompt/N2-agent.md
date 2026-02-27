# N2 — openmatch-agent (MAPLE Resonator)

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-agent/`
> **Depends on**: `openmatch-types`, `openmatch-core`, `tokio`

## Purpose

AI agent runtime for autonomous trading with built-in safety. Agents propose actions, risk gates validate, then execute. **No LLM-to-money path without verification.**

## Architecture

```
AgentRuntime
├── AgentRegistry: HashMap<AgentId, Box<dyn TradingAgent>>
├── RiskGate: validates every action before execution
├── Executor: submits validated actions to epoch controller
└── Monitor: tracks P&L, positions, risk metrics per agent

Resonance Flow:
  Presence → Coupling → Meaning → Intent → Commitment → Consequence
  (observe)  (connect)  (analyze) (decide) (risk-gate)  (execute)
```

## Agent Trait

```rust
#[async_trait]
trait TradingAgent: Send + Sync {
    /// Called each epoch during COLLECT phase
    async fn on_tick(&mut self, ctx: &AgentContext) -> Vec<AgentAction>;
    /// Called when a trade executes involving this agent's orders
    async fn on_trade(&mut self, trade: &Trade) -> Vec<AgentAction>;
    /// Called on orderbook changes
    async fn on_orderbook_update(&mut self, snapshot: &OrderBookSnapshot) -> Vec<AgentAction>;
    /// Agent metadata
    fn name(&self) -> &str;
    fn agent_id(&self) -> AgentId;
}

enum AgentAction {
    PlaceOrder { market: MarketPair, side: OrderSide, price: Decimal, qty: Decimal },
    CancelOrder { order_id: OrderId },
    CancelAll { market: MarketPair },
    DoNothing,
}

struct AgentContext {
    balances: HashMap<Asset, BalanceEntry>,
    open_orders: Vec<Order>,
    ticker: Ticker,
    epoch_id: EpochId,
    epoch_phase: EpochPhase,
    timestamp: DateTime<Utc>,
}
```

## 🔒 Agent Asset Protection (Security)

### RiskGate — Every action passes through before execution

```rust
struct RiskGate {
    limits: AgentRiskLimits,
    positions: HashMap<(AgentId, Asset), Decimal>,
    epoch_pnl: HashMap<AgentId, Decimal>,
}

struct AgentRiskLimits {
    /// Maximum total frozen balance per agent across all assets (in quote currency)
    max_total_exposure: Decimal,
    /// Maximum frozen per single asset
    max_asset_exposure: Decimal,
    /// Maximum number of open orders
    max_open_orders: usize,
    /// Maximum loss per epoch before agent is paused
    max_epoch_loss: Decimal,
    /// Maximum loss per day before agent is disabled
    max_daily_loss: Decimal,
    /// Maximum single order size
    max_order_size: Decimal,
    /// Minimum balance that must remain unfrozen (emergency reserve)
    min_available_reserve: Decimal,
    /// Rate limit: max orders per second
    max_orders_per_second: u32,
}
```

### Security Rules

1. **No raw fund access**: Agents interact through `AgentAction` enum only — no direct BalanceManager access
2. **Freeze ceiling**: `max_total_exposure` caps how much an agent can have frozen at once
3. **Loss circuit breaker**: If epoch P&L < -`max_epoch_loss`, agent is paused until manual review
4. **Emergency reserve**: `min_available_reserve` ensures withdrawal is always possible
5. **Action audit log**: Every AgentAction is logged with timestamp, agent_id, and risk gate decision
6. **Sandbox isolation**: Each agent runs in its own tokio task with resource limits (CPU, memory)
7. **No cross-agent access**: Agents cannot read other agents' state or orders

### Protection Flow
```
Agent proposes action
  → RiskGate.validate(action, agent_context)
    → Check exposure limits
    → Check loss limits (epoch + daily)
    → Check order rate limit
    → Check reserve requirement
    → IF ALL PASS → Executor.submit(action)
    → IF ANY FAIL → Log rejection + return error to agent
```

## Built-in Strategy Templates

1. **MarketMaker**: Maintains bid/ask spread, re-quotes after fills
2. **Arbitrageur**: Detects cross-market mispricing (future: cross-node)
3. **TWAPExecutor**: Time-weighted average price execution over N epochs

## Testing

1. Mock agent: verify on_tick called each epoch
2. Risk gate: exceed limits → action rejected
3. Loss circuit breaker: simulate losses → agent paused
4. Sandbox: agent panic → runtime continues
5. Action audit: verify complete log of all proposed + executed actions
