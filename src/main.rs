//! CLI producer -> bounded in-process queue -> single matcher thread.

use std::{
    env,
    error::Error,
    io::{self, BufRead},
    sync::mpsc::sync_channel,
    thread,
};

use lobme_rs::{Command, Engine, Order, Policy, Side};

fn parse(line: &str) -> Result<Option<Command>, String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }
    let parts: Vec<_> = line.split_whitespace().collect();
    let number = |value: &str| {
        value
            .parse::<u64>()
            .map_err(|_| format!("invalid number: {value}"))
    };
    match parts.as_slice() {
        [side, id, price, quantity] if *side == "buy" || *side == "sell" => {
            let side = if *side == "buy" {
                Side::Buy
            } else {
                Side::Sell
            };
            Ok(Some(Command::Place(Order {
                id: number(id)?,
                side,
                price: number(price)?,
                quantity: number(quantity)?,
            })))
        }
        ["cancel", id] => Ok(Some(Command::Cancel(number(id)?))),
        _ => Err("expected: buy ID PRICE QTY | sell ID PRICE QTY | cancel ID".into()),
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let policy = match env::args().nth(1).as_deref() {
        None | Some("fifo") => Policy::Fifo,
        Some("pro-rata") => Policy::ProRata,
        _ => return Err("usage: cargo run --release -- [fifo|pro-rata] < orders.txt".into()),
    };
    let (sender, receiver) = sync_channel(1024);
    let worker = thread::spawn(move || {
        let mut engine = Engine::new(policy);
        let mut events = Vec::new();
        for command in receiver {
            engine.apply(command, &mut events)?;
            println!("{command:?}");
            for event in &events {
                println!("  {event:?}");
            }
        }
        println!("Final book: {:#?}", engine.snapshot());
        Ok::<_, lobme_rs::EngineError>(())
    });
    for (line_number, line) in io::stdin().lock().lines().enumerate() {
        let line = line?;
        if let Some(command) = parse(&line)
            .map_err(|error| format!("line {}: {error}", line_number.saturating_add(1)))?
        {
            sender.send(command)?;
        }
    }
    drop(sender);
    worker
        .join()
        .map_err(|_| io::Error::other("matcher thread panicked"))?
        .map_err(|error| io::Error::other(format!("matcher error: {error:?}")))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    run()
}
