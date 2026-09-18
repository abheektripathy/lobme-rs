use lobme_rs::{Command, Engine, Event, Order, Policy, Side};

const fn order(id: u64, side: Side, price: u64, quantity: u64) -> Command {
    Command::Place(Order {
        id,
        side,
        price,
        quantity,
    })
}

fn apply(engine: &mut Engine, command: Command) -> Vec<Event> {
    let mut events = Vec::new();
    assert_eq!(engine.apply(command, &mut events), Ok(()));
    events
}

#[test]
fn non_crossing_order_rests() {
    let mut engine = Engine::new(Policy::Fifo);
    assert_eq!(
        apply(&mut engine, order(1, Side::Sell, 100, 10)),
        vec![Event::Rested {
            id: 1,
            remaining: 10
        }]
    );
    assert!(engine.snapshot().bids.is_empty());
    assert_eq!(engine.snapshot().asks[0].orders, vec![(1, 10)]);
}

#[test]
fn fifo_and_partial_fill() {
    let mut engine = Engine::new(Policy::Fifo);
    apply(&mut engine, order(1, Side::Sell, 100, 5));
    apply(&mut engine, order(2, Side::Sell, 100, 5));
    assert_eq!(
        apply(&mut engine, order(3, Side::Buy, 100, 7)),
        vec![
            Event::Trade {
                maker: 1,
                taker: 3,
                price: 100,
                quantity: 5
            },
            Event::Trade {
                maker: 2,
                taker: 3,
                price: 100,
                quantity: 2
            },
        ]
    );
    assert_eq!(engine.snapshot().asks[0].orders, vec![(2, 3)]);
}

#[test]
fn better_price_beats_earlier_order() {
    let mut engine = Engine::new(Policy::Fifo);
    apply(&mut engine, order(1, Side::Sell, 101, 1));
    apply(&mut engine, order(2, Side::Sell, 100, 1));
    assert_eq!(
        apply(&mut engine, order(3, Side::Buy, 102, 2)),
        vec![
            Event::Trade {
                maker: 2,
                taker: 3,
                price: 100,
                quantity: 1
            },
            Event::Trade {
                maker: 1,
                taker: 3,
                price: 101,
                quantity: 1
            },
        ]
    );
}

#[test]
fn pro_rata_splits_same_price_and_remainder_is_fifo() {
    let mut engine = Engine::new(Policy::ProRata);
    for id in 1..=3 {
        apply(&mut engine, order(id, Side::Sell, 100, 5));
    }
    assert_eq!(
        apply(&mut engine, order(4, Side::Buy, 100, 5)),
        vec![
            Event::Trade {
                maker: 1,
                taker: 4,
                price: 100,
                quantity: 2
            },
            Event::Trade {
                maker: 2,
                taker: 4,
                price: 100,
                quantity: 2
            },
            Event::Trade {
                maker: 3,
                taker: 4,
                price: 100,
                quantity: 1
            },
        ]
    );
    assert_eq!(
        engine.snapshot().asks[0].orders,
        vec![(1, 3), (2, 3), (3, 4)]
    );
}

#[test]
fn pro_rata_does_not_skip_better_price() {
    let mut engine = Engine::new(Policy::ProRata);
    apply(&mut engine, order(1, Side::Sell, 100, 2));
    apply(&mut engine, order(2, Side::Sell, 101, 10));
    assert_eq!(
        apply(&mut engine, order(3, Side::Buy, 101, 3)),
        vec![
            Event::Trade {
                maker: 1,
                taker: 3,
                price: 100,
                quantity: 2
            },
            Event::Trade {
                maker: 2,
                taker: 3,
                price: 101,
                quantity: 1
            },
        ]
    );
}

#[test]
fn cancellation_and_duplicate_ids() {
    let mut engine = Engine::new(Policy::Fifo);
    apply(&mut engine, order(1, Side::Buy, 99, 3));
    assert_eq!(
        apply(&mut engine, Command::Cancel(1)),
        vec![Event::Cancelled { id: 1 }]
    );
    assert!(engine.snapshot().bids.is_empty());
    assert_eq!(
        apply(&mut engine, order(1, Side::Buy, 99, 3)),
        vec![Event::Rejected {
            id: 1,
            reason: "order ID already used"
        },]
    );
}

#[test]
fn sell_taker_executes_at_resting_bid_price() {
    let mut engine = Engine::new(Policy::Fifo);
    apply(&mut engine, order(1, Side::Buy, 102, 4));
    assert_eq!(
        apply(&mut engine, order(2, Side::Sell, 100, 2)),
        vec![Event::Trade {
            maker: 1,
            taker: 2,
            price: 102,
            quantity: 2
        },]
    );
    assert_eq!(engine.snapshot().bids[0].orders, vec![(1, 2)]);
}

#[test]
fn unfilled_limit_rests_at_its_own_price() {
    let mut engine = Engine::new(Policy::Fifo);
    apply(&mut engine, order(1, Side::Sell, 100, 2));
    assert_eq!(
        apply(&mut engine, order(2, Side::Buy, 101, 5)),
        vec![
            Event::Trade {
                maker: 1,
                taker: 2,
                price: 100,
                quantity: 2
            },
            Event::Rested {
                id: 2,
                remaining: 3
            },
        ]
    );
    assert_eq!(engine.snapshot().bids[0].price, 101);
    assert_eq!(engine.snapshot().bids[0].orders, vec![(2, 3)]);
}

#[test]
fn cancelling_middle_order_preserves_fifo() {
    let mut engine = Engine::new(Policy::Fifo);
    for id in 1..=3 {
        apply(&mut engine, order(id, Side::Sell, 100, 1));
    }
    apply(&mut engine, Command::Cancel(2));
    assert_eq!(
        apply(&mut engine, order(4, Side::Buy, 100, 2)),
        vec![
            Event::Trade {
                maker: 1,
                taker: 4,
                price: 100,
                quantity: 1
            },
            Event::Trade {
                maker: 3,
                taker: 4,
                price: 100,
                quantity: 1
            },
        ]
    );
}

#[test]
fn pro_rata_conserves_large_integer_lots() {
    let mut engine = Engine::new(Policy::ProRata);
    apply(&mut engine, order(1, Side::Sell, 100, u64::MAX));
    apply(&mut engine, order(2, Side::Sell, 100, u64::MAX));
    let events = apply(&mut engine, order(3, Side::Buy, 100, u64::MAX));
    let filled = events
        .iter()
        .filter_map(|event| match event {
            Event::Trade { quantity, .. } => Some(u128::from(*quantity)),
            _ => None,
        })
        .sum::<u128>();
    assert_eq!(filled, u128::from(u64::MAX));
    assert_eq!(engine.snapshot().asks[0].orders.len(), 2);
}
