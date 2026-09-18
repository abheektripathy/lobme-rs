# lobme-rs

A small Rust limit-order-book matching POC. It processes integer tick prices and lot quantities in command order, with price priority first and a selectable allocation rule within the best price level.

Still a work in progress; the scope below is what exists today.

## Try it

```sh
cargo test
cargo run --release -- fifo < examples/orders.txt
cargo run --release -- pro-rata < examples/orders.txt
```

Input is one command per line; `#` starts a comment:

```text
sell 1 100 5
sell 2 100 5
buy 3 100 5
cancel 2
```

The fields are `side ID PRICE_TICKS QUANTITY_LOTS`. Orders at a price remain in arrival order. A new order trades at the **resting maker's price**; any unfilled quantity rests at its own limit price. Each command produces ordered events, and the CLI prints the final book. IDs cannot be reused after an accepted order is filled or cancelled.

At one price:

- `fifo` fills the oldest resting order first.
- `pro-rata` splits the executable quantity in proportion to each resting order's remaining lots. Fractional lots are rounded down, then leftover lots go to older eligible orders first. This is a POC rule, not a claim to reproduce a particular exchange's pro-rata algorithm.

The CLI places commands into a bounded, **in-process** `std::sync::mpsc::sync_channel`; one worker thread owns and mutates the book. This is not inter-process shared memory or a network server. Backpressure blocks the producer when the 1,024-command queue is full. The matcher itself does no I/O or threading.

## Scope

Included: one market, limit orders, partial fills, cancellation, price priority, FIFO/pro-rata at a price, deterministic events, and tests. Not included: accounts, balances, risk, clearing, liquidation, mark price, TP/SL, live feeds, persistence, or recovery. A trade event is not a cleared/settled trade.

[`docs/future-exchange-plan.excalidraw`](docs/future-exchange-plan.excalidraw) preserves the original sketch as a *future-plan draft*, not the implemented architecture. In particular, it depicts a gateway and clearinghouse that do not exist in this POC.

Performance claims require a fixed workload and measured comparison. This version uses standard-library `BTreeMap` price levels and `VecDeque` orders; cancellation scans one price level. No HFT or end-to-end latency claim is made.

For a first matcher-only batch comparison (not per-order p99):

```sh
cargo run --release --example bench -- fifo
cargo run --release --example bench -- pro-rata
```

The benchmark prebuilds 40,000 identical commands for both policies, seeds 32 asks, and times only `Engine::apply` plus the cheap event-count check. It excludes CLI parsing, queueing, printing, and book snapshots. Results depend on CPU, build and workload; these policies perform different amounts of work and produce different numbers of fills.
