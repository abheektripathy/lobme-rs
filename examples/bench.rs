//! Synthetic matcher-only batch timing; intentionally not a latency percentile.

use std::{env, error::Error, time::Instant};

use lobme_rs::{Command, Engine, Order, Policy, Side};

const fn place(id: u64, side: Side, quantity: u64) -> Command {
    Command::Place(Order {
        id,
        side,
        price: 100,
        quantity,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let policy = match env::args().nth(1).as_deref() {
        Some("fifo") => Policy::Fifo,
        Some("pro-rata") => Policy::ProRata,
        _ => return Err("usage: cargo run --release --example bench -- [fifo|pro-rata]".into()),
    };
    let mut engine = Engine::new(policy);
    let mut events = Vec::with_capacity(64);
    for id in 1..=32 {
        engine
            .apply(place(id, Side::Sell, 10), &mut events)
            .map_err(|e| format!("{e:?}"))?;
    }
    let mut commands = Vec::with_capacity(40_000);
    for n in 0_u64..20_000 {
        let id = n.saturating_mul(2).saturating_add(33);
        commands.push(place(id, Side::Sell, 16));
        commands.push(place(id.saturating_add(1), Side::Buy, 16));
    }

    let start = Instant::now();
    let mut events_emitted = 0_u64;
    for command in commands.iter().copied() {
        engine
            .apply(command, &mut events)
            .map_err(|e| format!("{e:?}"))?;
        events_emitted = events_emitted.saturating_add(u64::try_from(events.len())?);
    }
    let elapsed = start.elapsed();
    let average_ns = elapsed
        .as_nanos()
        .checked_div(u128::try_from(commands.len())?)
        .unwrap_or(0);
    let final_book = std::hint::black_box(engine.snapshot());
    println!(
        "{policy:?}: {} commands, {events_emitted} events, {average_ns} ns/command batch average, final ask levels: {}",
        commands.len(),
        final_book.asks.len()
    );
    Ok(())
}
