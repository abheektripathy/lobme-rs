# `lob-matching-engine` technical implementation specification

- Status: implementation contract
- Target: `0.1.0`
- Engine: one in-memory limit order book
- Language: safe Rust 2024

## 1. Purpose

This document defines the public interfaces, matching behaviour, internal state, tests, CLI tools, strategy harness,
and benchmarks required for version `0.1.0`.

The engine accepts ordered commands and produces ordered events:

```text
Command + current Engine state -> Engine::apply -> Events + new Engine state
```

The words **MUST**, **SHOULD**, and **MAY** describe required, recommended, and optional behaviour.

## 2. Version 0.1 scope

The implementation MUST support:

- one order book representing one market;
- integer price ticks and quantity lots;
- GTC limit orders;
- IOC limit orders;
- market orders;
- cancellation by order ID;
- price priority;
- FIFO priority at the same price;
- partial fills;
- maker-price execution;
- deterministic events and snapshots;
- scenario verification;
- deterministic synthetic simulation;
- a synthetic strategy integration test;
- matching benchmarks.

The implementation MUST NOT include:

- Tokio or async code;
- network or exchange connections;
- persistence or a database;
- multiple assets inside one `Engine`;
- balances, margin, liquidations, or settlement;
- post-only, fill-or-kill, replace, or self-trade prevention;
- floating-point numbers in the engine;
- unsafe or lock-free code;
- historical market replay or claims of strategy profitability.

## 3. Repository layout

Implement files in this order. Do not create later-phase files before their phase begins.

```text
src/
  lib.rs          Public exports only
  model.rs        Commands, events, numeric types and snapshots
  engine.rs       Book state and matching
  scenario.rs     Phase 2: JSONL scenario verification
  generator.rs    Phase 3: deterministic synthetic commands
  strategy.rs     Phase 5: synthetic strategy and portfolio
  main.rs         Phase 2: CLI
tests/
  matching.rs     Public behaviour tests
scenarios/
  price-time-priority.jsonl
  partial-fill.jsonl
benches/
  engine.rs       Phase 4
scripts/
  check.sh
Cargo.toml
README.md
SPEC.md
```

## 4. Required changes to the current model

The current crate has `Price`, `Amount`, `Assets`, `PriceLevel`, and `RestingOrder` in `model.rs`.

Before implementing the matcher:

- rename `Amount` to `Quantity`;
- remove `Assets` from the engine model;
- move `PriceLevel` and `RestingOrder` out of `model.rs` and make them private engine implementation types;
- add `OwnerId`, commands, order kinds, events, rejection reasons, and snapshots;
- keep the filename singular: `model.rs` is loaded with `mod model;`.

One `Engine` represents one market. Multiple assets, if ever required, belong in a later coordinator such as
`HashMap<InstrumentId, Engine>`, not in every resting order.

## 5. Public model interface

`src/model.rs` MUST expose the following logical API. Field visibility may be implemented directly or through
constructors and accessors, but callers MUST be able to construct commands and inspect events without accessing engine
internals.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Price(u64);

impl Price {
    pub const fn from_ticks(ticks: u64) -> Self;
    pub const fn ticks(self) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quantity(u64);

impl Quantity {
    pub const ZERO: Self;

    pub const fn from_lots(lots: u64) -> Self;
    pub const fn lots(self) -> u64;
    pub const fn is_zero(self) -> bool;
    pub const fn min(self, other: Self) -> Self;
    pub const fn checked_add(self, other: Self) -> Option<Self>;
    pub const fn checked_sub(self, other: Self) -> Option<Self>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId(u64);

impl OrderId {
    pub const fn new(value: u64) -> Self;
    pub const fn value(self) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerId(u32);

impl OwnerId {
    pub const fn new(value: u32) -> Self;
    pub const fn value(self) -> u32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub const fn opposite(self) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    Gtc,
    Ioc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    Limit {
        price: Price,
        time_in_force: TimeInForce,
    },
    Market,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewOrder {
    pub id: OrderId,
    pub owner: OwnerId,
    pub side: Side,
    pub quantity: Quantity,
    pub kind: OrderKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Submit(NewOrder),
    Cancel { order_id: OrderId },
}
```

`Price(0)` is representable because these types are storage units, not validation boundaries. A zero limit price MUST be
rejected by `Engine::apply`. A market order has no price.

## 6. Event interface

Events are the only execution output. Event order is part of the public contract.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    ZeroQuantity,
    ZeroLimitPrice,
    DuplicateOrderId,
    UnknownOrderId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    UserRequested,
    ImmediateOrCancel,
    NoLiquidity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Accepted {
        order_id: OrderId,
    },
    Trade {
        sequence: u64,
        maker_id: OrderId,
        maker_owner: OwnerId,
        taker_id: OrderId,
        taker_owner: OwnerId,
        taker_side: Side,
        price: Price,
        quantity: Quantity,
    },
    Rested {
        order_id: OrderId,
        remaining: Quantity,
    },
    Cancelled {
        order_id: OrderId,
        remaining: Quantity,
        reason: CancelReason,
    },
    Rejected {
        order_id: OrderId,
        reason: RejectReason,
    },
}
```

`Trade::taker_side` MUST be present. It makes a trade self-contained: if the strategy is the maker, its side is
`taker_side.opposite()`.

### 6.1 Event ordering

For an accepted submission, events MUST appear in this order:

```text
Accepted
zero or more Trade events
optional Rested or Cancelled remainder event
```

For a rejected submission or cancellation, the only event MUST be `Rejected`.

For a successful explicit cancellation, the only event MUST be `Cancelled { reason: UserRequested }`.

## 7. Snapshot interface

Snapshots are for tests, CLI output, and strategy decisions. They MUST NOT be created inside timed matching
benchmarks.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelSnapshot {
    pub price: Price,
    pub total_quantity: Quantity,
    pub order_ids: Vec<OrderId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSnapshot {
    pub bids: Vec<LevelSnapshot>,
    pub asks: Vec<LevelSnapshot>,
}
```

Snapshot ordering MUST be:

- bids from highest price to lowest price;
- asks from lowest price to highest price;
- order IDs in FIFO order within each price.

`snapshot(0)` MUST return empty bid and ask vectors. `snapshot(n)` MUST return at most `n` price levels per side.

## 8. Engine public interface

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    ArithmeticOverflow,
    InvariantViolation,
}

pub struct Engine {
    // private fields
}

impl Engine {
    pub fn new() -> Self;

    pub fn apply(
        &mut self,
        command: Command,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError>;

    pub fn snapshot(&self, max_levels: usize) -> BookSnapshot;

    pub fn order_count(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}

impl Default for Engine {
    fn default() -> Self;
}
```

`next_trade_sequence` MUST start at `1`. Each emitted trade uses the current value, then advances it with
`checked_add(1)`.

`Engine::apply` MUST:

1. clear the caller-owned event buffer;
2. process exactly one command synchronously;
3. append ordered events;
4. return only after the state transition is complete;
5. perform no file, network, logging, clock, async, or strategy work.

The caller SHOULD reserve event capacity once and reuse the same vector.

Invalid user commands produce `Event::Rejected` and return `Ok(())`. `EngineError` is reserved for internal arithmetic
or state corruption. On `EngineError`, the event buffer MUST be cleared and the caller MUST stop using that engine.

## 9. Internal state

`src/engine.rs` MUST use this correctness-first representation:

```rust
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

pub struct Engine {
    bids: BTreeMap<Price, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    orders: HashMap<OrderId, RestingOrder>,
    seen_ids: HashSet<OrderId>,
    next_trade_sequence: u64,
}

struct PriceLevel {
    order_ids: VecDeque<OrderId>,
    total_quantity: Quantity,
}

struct RestingOrder {
    id: OrderId,
    owner: OwnerId,
    side: Side,
    price: Price,
    remaining: Quantity,
}
```

The implementation MUST NOT duplicate full `RestingOrder` values inside price-level queues. `orders` owns order state;
price levels hold IDs and aggregate quantity.

Cancellation MAY scan the `VecDeque` for the located order. This is intentionally O(number of orders at one price).
Do not add an arena or linked-list index until a cancellation benchmark demonstrates the need.

## 10. Internal function contract

The implementation SHOULD be split into these functions. Names may differ, but responsibilities MUST remain separate.

```rust
impl Engine {
    fn submit(
        &mut self,
        order: NewOrder,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError>;

    fn cancel(
        &mut self,
        order_id: OrderId,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError>;

    fn validate_submission(&self, order: &NewOrder) -> Result<(), RejectReason>;

    fn best_opposite_price(&self, taker_side: Side) -> Option<Price>;

    fn crosses(taker_side: Side, kind: OrderKind, maker_price: Price) -> bool;

    fn match_incoming(
        &mut self,
        order: &NewOrder,
        remaining: Quantity,
        events: &mut Vec<Event>,
    ) -> Result<Quantity, EngineError>;

    fn execute_match(
        &mut self,
        taker: &NewOrder,
        taker_remaining: Quantity,
        maker_id: OrderId,
        maker_price: Price,
        events: &mut Vec<Event>,
    ) -> Result<Quantity, EngineError>;

    fn rest(
        &mut self,
        order: &NewOrder,
        remaining: Quantity,
    ) -> Result<(), EngineError>;

    fn remove_from_level(
        &mut self,
        side: Side,
        price: Price,
        order_id: OrderId,
        quantity: Quantity,
    ) -> Result<(), EngineError>;

    fn check_invariants(&self) -> Result<(), EngineError>;
}
```

Avoid helpers that return long-lived mutable references to whole books. Read the best price and maker ID in a short
immutable scope, copy those small values, then mutate the required fields. This keeps borrow lifetimes narrow.

## 11. Command validation

Submission validation MUST run before any state mutation in this order:

1. reject zero quantity;
2. reject a zero limit price;
3. reject an ID already present in `seen_ids`.

The first failed rule determines `RejectReason`.

A rejected submission MUST NOT add its ID to `seen_ids`. An accepted ID MUST remain in `seen_ids` after fill or
cancellation and can never be reused by that engine.

Cancelling an ID absent from `orders` MUST produce `UnknownOrderId` and leave state unchanged. This includes IDs that
previously filled or were cancelled.

## 12. Matching algorithm

### 12.1 Incoming buy

Repeat while quantity remains:

1. read the lowest ask with `BTreeMap::first_key_value`;
2. stop if no ask exists;
3. stop if a limit price is below the ask;
4. read the oldest maker ID at that level;
5. fill `min(taker remaining, maker remaining)`;
6. execute at the maker's resting price;
7. pre-compute the next trade sequence with `checked_add(1)`;
8. subtract the fill from maker, taker, and level totals using checked arithmetic;
9. if the maker is full, remove it from the queue and `orders`;
10. if the level is empty, remove the level;
11. emit one `Trade`, store the pre-computed next sequence, and continue.

### 12.2 Incoming sell

The sell algorithm mirrors the buy algorithm against the highest bid from `BTreeMap::last_key_value`. A sell limit
crosses when its price is less than or equal to the best bid.

### 12.3 Remaining quantity

After matching:

- remaining GTC limit quantity MUST rest at the back of its price queue and emit `Rested`;
- remaining IOC limit quantity MUST NOT rest and MUST emit `Cancelled { reason: ImmediateOrCancel }`;
- remaining market quantity MUST NOT rest and MUST emit `Cancelled { reason: NoLiquidity }`;
- zero remaining quantity emits no final `Rested` or `Cancelled` event.

Self-trading is allowed in `0.1.0`; self-trade prevention is explicitly deferred.

## 13. Cancellation algorithm

For a known resting order:

1. copy its side, price, and remaining quantity from `orders`;
2. find its ID in that price level's `VecDeque`;
3. remove the ID;
4. subtract its quantity from the level total;
5. remove the order from `orders`;
6. remove the price level if it is empty;
7. emit `Cancelled { reason: UserRequested }`.

If `orders` says an order exists but its price queue does not contain it, return `EngineError::InvariantViolation` and
stop the engine.

## 14. Required invariants

`check_invariants` MUST verify:

- the best bid is lower than the best ask when both exist;
- no resting order has zero quantity;
- no price level has zero total quantity;
- every resting order appears exactly once in one price queue;
- every queued ID exists in `orders`;
- queued side and price match the stored order;
- each price-level total equals the checked sum of its resting orders;
- no empty price level exists;
- each ID in `orders` exists in `seen_ids`;
- `next_trade_sequence` is non-zero.

Full invariant scans MUST run in tests. They MUST NOT run in release benchmarks.

## 15. Library exports

`src/lib.rs` MUST remain small:

```rust
mod engine;
mod model;

pub use engine::{Engine, EngineError};
pub use model::{
    BookSnapshot, CancelReason, Command, Event, LevelSnapshot, NewOrder, OrderId,
    OrderKind, OwnerId, Price, Quantity, RejectReason, Side, TimeInForce,
};
```

Do not export `PriceLevel` or `RestingOrder`; they are internal representations.

## 16. Core test contract

`tests/matching.rs` MUST test the public API. Each test should issue commands, compare exact events, and compare a
snapshot. Use small helper constructors only inside the test file.

| Test name | Required assertion |
|---|---|
| `non_crossing_limit_rests` | One order rests at its submitted price. |
| `crossing_order_executes_at_maker_price` | Trade price is the maker's price, not the taker's limit. |
| `better_price_executes_first` | Best price wins before time priority. |
| `same_price_executes_fifo` | Older order fills before newer order. |
| `partial_maker_keeps_priority` | Partial maker remains at queue front. |
| `taker_sweeps_multiple_levels` | Trades are emitted in price then FIFO order. |
| `trade_sequence_increments_once_per_trade` | The first trade is sequence 1 and every trade increments by one. |
| `gtc_remainder_rests` | Unfilled GTC remainder rests. |
| `ioc_remainder_is_cancelled` | IOC remainder does not enter the book. |
| `market_remainder_is_cancelled` | Market remainder does not enter the book. |
| `cancel_removes_order_and_empty_level` | Order, quantity, and empty price level disappear. |
| `cancel_preserves_other_fifo_orders` | Cancelling a middle order does not reorder others. |
| `zero_quantity_is_rejected_without_mutation` | One rejection event; snapshot unchanged. |
| `zero_limit_price_is_rejected_without_mutation` | One rejection event; snapshot unchanged. |
| `accepted_id_cannot_be_reused` | ID remains consumed after fill or cancellation. |
| `rejected_id_can_be_corrected_and_resubmitted` | A validation rejection does not consume the ID. |
| `unknown_cancel_is_rejected_without_mutation` | Snapshot and counts remain unchanged. |
| `same_commands_are_deterministic` | Two engines produce identical events and snapshots. |

At least one internal test MUST run a deterministic mixed command stream and call `check_invariants` after every
command.

## 17. Scenario verifier

Add this only after all core tests pass.

The CLI command is:

```bash
lob verify scenarios/price-time-priority.jsonl
```

The scenario module interface SHOULD be:

```rust
pub fn verify_scenario<R: std::io::BufRead>(
    reader: R,
) -> Result<VerificationReport, ScenarioError>;

pub struct VerificationReport {
    pub command_count: u64,
    pub event_count: u64,
}
```

Serde wire types MUST remain private to `scenario.rs`. Do not add Serde derives or a Serde dependency to the core
engine model.

A scenario contains command records followed by exactly one expected result:

```json
{"type":"command","command":{"submit":{"id":1,"owner":1,"side":"sell","quantity":20,"kind":{"limit":{"price":10000,"time_in_force":"gtc"}}}}}
{"type":"command","command":{"submit":{"id":2,"owner":2,"side":"sell","quantity":10,"kind":{"limit":{"price":10000,"time_in_force":"gtc"}}}}}
{"type":"command","command":{"submit":{"id":3,"owner":3,"side":"buy","quantity":25,"kind":"market"}}}
{"type":"expect","trades":[{"maker":1,"taker":3,"taker_side":"buy","price":10000,"quantity":20},{"maker":2,"taker":3,"taker_side":"buy","price":10000,"quantity":5}],"asks":[{"price":10000,"quantity":5,"orders":[2]}],"bids":[]}
```

The verifier MUST:

- report malformed JSON with file and line context;
- reject commands after the expect record;
- reject missing or multiple expect records;
- apply every command through the real `Engine::apply` method;
- compare exact trade order and exact final snapshot;
- print the first useful mismatch;
- return success on a match and failure on a mismatch.

`main` SHOULD return `std::process::ExitCode`; it MUST NOT call `std::process::exit`, matching the crate's lint policy.

Once the CLI exists, `Cargo.toml` MUST contain:

```toml
[[bin]]
name = "lob"
path = "src/main.rs"
```

## 18. Deterministic generator and simulator

Add this after scenario verification.

```rust
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    pub fn new(seed: u64) -> Result<Self, GeneratorError>;
    pub fn next_u64(&mut self) -> u64;
}

pub fn generate_commands(config: &GeneratorConfig) -> Result<Vec<Command>, GeneratorError>;

pub fn simulate(commands: &[Command]) -> Result<SimulationReport, EngineError>;
```

Seed zero MUST be rejected because xorshift would remain zero forever. The xorshift bit operations MUST be fixed and
documented so the same seed generates the same stream across machines.

The generator MUST create:

- initial bid and ask depth around one reference price;
- non-crossing limits;
- crossing limits;
- market orders;
- cancellations of known resting IDs.

The generator MUST run before timed benchmarks. The simulation report MUST include seed, command mix, initial book
shape, command counts, event counts, and deterministic final-state and event hashes.

Hashes MUST consume ordered events and ordered snapshot fields using one fixed documented algorithm such as FNV-1a.
Do not hash `HashMap` iteration order and do not treat Rust's `DefaultHasher` implementation as a stable file format.

Do not describe the generator as a realistic market model.

## 19. Benchmark contract

Add Criterion only after correctness and deterministic simulation pass.

`benches/engine.rs` MUST measure:

- rest one non-crossing limit;
- cross one resting maker;
- partially fill one maker;
- sweep five price levels;
- cancel near the front of a populated level;
- cancel near the back of a populated level;
- process a pre-generated mixed stream.

The timed region MUST include only calls to `Engine::apply`. It MUST exclude:

- command generation;
- JSON parsing;
- snapshots and invariant scans;
- logging and printing;
- strategy calculations;
- report formatting.

Every benchmark report MUST state CPU, operating system, Rust version, release profile, commit SHA, command mix, book
shape, warm-up, and sample count.

A separate batch harness MAY report fixed-batch p50 and p99. A batch duration divided by command count MUST NOT be
called per-order p99.

## 20. Synthetic strategy harness

Add one built-in strategy after benchmarks exist. Do not add a strategy trait or plugin registry for one strategy.

```rust
pub struct StrategyConfig {
    pub seed: u64,
    pub background_commands: u64,
    pub decision_interval: u64,
    pub order_quantity: Quantity,
    pub max_position: Quantity,
    pub fee_bps: u32,
}

pub struct Portfolio {
    pub position: Quantity,
    pub cash: i128,
    pub fees_paid: u128,
    pub marked_equity: i128,
    pub max_drawdown: u128,
}

pub struct StrategyReport {
    pub commands: u64,
    pub trades: u64,
    pub position: Quantity,
    pub cash: i128,
    pub fees_paid: u128,
    pub marked_equity: i128,
    pub max_drawdown: u128,
}

pub fn run_strategy(config: &StrategyConfig) -> Result<StrategyReport, StrategyError>;

fn decide(
    snapshot: &BookSnapshot,
    portfolio: &Portfolio,
    config: &StrategyConfig,
    next_order_id: OrderId,
) -> Result<Option<NewOrder>, StrategyError>;

fn apply_strategy_trade(
    portfolio: &mut Portfolio,
    event: &Event,
    strategy_owner: OwnerId,
    fee_bps: u32,
) -> Result<(), StrategyError>;
```

The strategy loop MUST:

1. apply deterministic background commands through `Engine::apply`;
2. after each `decision_interval`, request the top five levels;
3. calculate imbalance using checked integer arithmetic;
4. buy when imbalance is at least 6,500 basis points;
5. exit the long position when imbalance is at most 4,500 basis points;
6. remain long-only and within `max_position`;
7. create a normal market `NewOrder` owned by a reserved strategy owner ID;
8. send that order through the same `Engine::apply` path;
9. update the portfolio only from strategy-owned `Trade` events;
10. mark a long position using the current best bid;
11. produce a report after the configured background command count.

Use `u128` checked arithmetic for notional and fee calculations before converting to `i128`. Never use `f64` for
cash, fees, imbalance, or PnL.

For a strategy-owned taker fill, accounting uses `taker_side`. For a strategy-owned maker fill, accounting uses
`taker_side.opposite()`. A trade where both maker and taker are the strategy owner MUST return a strategy error in
version `0.1.0`. The synthetic fee is `notional * fee_bps / 10_000`, rounded down through integer division.

Required strategy assertions:

- the same config produces the same report;
- position never exceeds `max_position`;
- every portfolio change corresponds to a strategy-owned `Trade` event;
- the strategy cannot provide its execution price;
- fees are applied to every strategy fill;
- no future command or book state is inspected.

The CLI and README MUST label this result **synthetic**. It tests integration, accounting, and limits; it does not test
historical profitability.

## 21. Check script

`scripts/check.sh` MUST be the one local and CI entry point:

```bash
#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets

if [[ -f scenarios/price-time-priority.jsonl ]]; then
  cargo run --quiet --bin lob -- verify scenarios/price-time-priority.jsonl
  cargo run --quiet --bin lob -- verify scenarios/partial-fill.jsonl
fi
```

The file MUST be executable. CI SHOULD call only `./scripts/check.sh` so local and CI validation cannot drift.

The current lint configuration denies unchecked arithmetic, panics, `unwrap`, `expect`, direct indexing, and process
exit in production code. Engine arithmetic MUST therefore use checked operations and propagate typed errors. Test-only
allowances already exist in `clippy.toml`.

Pure constructors and query functions SHOULD use `#[must_use]`. Public functions returning `Result` MUST document their
failure conditions with an `# Errors` section. Numeric conversions MUST use `From` or `TryFrom`, not unchecked casts.

## 22. Implementation order

### Phase 1: correct engine

1. Replace the placeholder model.
2. Implement numeric helpers.
3. Implement snapshots.
4. Implement non-crossing GTC rest.
5. Implement crossing and partial fills.
6. Implement cancellation.
7. Implement all core tests and invariant checks.

Do not continue until `scripts/check.sh` passes the core crate.

### Phase 2: scenario verification

1. Add Serde, Serde JSON, and the small CLI.
2. Implement JSONL parsing and exact comparison.
3. Add the two initial scenario files.

### Phase 3: deterministic simulation

1. Add `XorShift64`.
2. Pre-generate a repeatable command stream.
3. Report deterministic event and state hashes.

### Phase 4: baseline benchmarks

1. Add Criterion as a development dependency.
2. Publish the correctness-first baseline.
3. Profile before changing data structures.

### Phase 5: strategy integration test

1. Add the imbalance decision function.
2. Add position and fee accounting.
3. Route strategy orders through `Engine::apply`.
4. Add deterministic report assertions.

### Phase 6: measured optimization

1. Identify one measured bottleneck.
2. Change one code path or data structure.
3. Run the full check script.
4. Re-run identical benchmarks.
5. Keep the change only if improvement is repeatable.

## 23. Version 0.1 acceptance

Version `0.1.0` is complete when:

- `scripts/check.sh` passes;
- every core test in section 16 exists and passes;
- scenarios produce exact expected events and final books;
- a million-command run is deterministic across two executions;
- the strategy respects its position limit and is reproducible;
- benchmark input is generated outside the timed region;
- README results include machine and workload context;
- the engine contains no async, I/O, logging, locks, clocks, or strategy code;
- no unmeasured low-latency, HFT-grade, or profitability claim is published.

## 24. Deferred until evidence requires it

- O(1) cancellation through an indexed arena;
- reusable order slots or custom allocators;
- tick-indexed price arrays;
- post-only, FOK, replace, and self-trade prevention;
- multiple books and engine threads;
- persistence, snapshots, replication, and recovery;
- historical feeds, queue-position models, and latency models;
- TUI and live exchange connections.
