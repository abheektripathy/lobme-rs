//! One synchronous owner for one in-memory order book. No sockets or I/O here.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::{BookSnapshot, Command, Event, LevelSnapshot, Order, Policy, Side};

#[derive(Debug, Eq, PartialEq)]
pub enum EngineError {
    ArithmeticOverflow,
    CorruptBook,
}

pub struct Engine {
    policy: Policy,
    bids: BTreeMap<u64, VecDeque<Order>>,
    asks: BTreeMap<u64, VecDeque<Order>>,
    active: HashMap<u64, (Side, u64)>, // ID -> (side, price), for cancellation
    seen: HashSet<u64>,                // accepted IDs cannot be reused
    allocations: Vec<(usize, u64)>,    // reused across commands
}

impl Engine {
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            active: HashMap::new(),
            seen: HashSet::new(),
            allocations: Vec::new(),
        }
    }

    /// Process one command completely. The caller owns and may reuse `events`.
    /// Stop using this engine if an internal error is returned.
    ///
    /// # Errors
    /// Returns an error if book state is inconsistent or arithmetic overflows.
    pub fn apply(&mut self, command: Command, events: &mut Vec<Event>) -> Result<(), EngineError> {
        events.clear();
        match command {
            Command::Place(order) => self.place(order, events),
            Command::Cancel(id) => self.cancel(id, events),
        }
    }

    fn place(&mut self, mut incoming: Order, events: &mut Vec<Event>) -> Result<(), EngineError> {
        if incoming.quantity == 0 || incoming.price == 0 {
            events.push(Event::Rejected {
                id: incoming.id,
                reason: "price and quantity must be positive",
            });
            return Ok(());
        }
        if !self.seen.insert(incoming.id) {
            events.push(Event::Rejected {
                id: incoming.id,
                reason: "order ID already used",
            });
            return Ok(());
        }

        while incoming.quantity > 0 {
            let best = match incoming.side {
                Side::Buy => self.asks.first_key_value().map(|(p, _)| *p),
                Side::Sell => self.bids.last_key_value().map(|(p, _)| *p),
            };
            let Some(price) = best else { break };
            let crosses = match incoming.side {
                Side::Buy => incoming.price >= price,
                Side::Sell => incoming.price <= price,
            };
            if !crosses {
                break;
            }

            let opposite = match incoming.side {
                Side::Buy => &mut self.asks,
                Side::Sell => &mut self.bids,
            };
            let level = opposite.get_mut(&price).ok_or(EngineError::CorruptBook)?;
            allocate(self.policy, level, incoming.quantity, &mut self.allocations)?;
            if self.allocations.is_empty() {
                return Err(EngineError::CorruptBook);
            }

            for (index, quantity) in self.allocations.drain(..) {
                if quantity == 0 {
                    continue;
                }
                let maker = level.get_mut(index).ok_or(EngineError::CorruptBook)?;
                maker.quantity = maker
                    .quantity
                    .checked_sub(quantity)
                    .ok_or(EngineError::CorruptBook)?;
                incoming.quantity = incoming
                    .quantity
                    .checked_sub(quantity)
                    .ok_or(EngineError::CorruptBook)?;
                events.push(Event::Trade {
                    maker: maker.id,
                    taker: incoming.id,
                    price,
                    quantity,
                });
                if maker.quantity == 0 {
                    self.active.remove(&maker.id);
                }
            }
            match self.policy {
                Policy::Fifo => {
                    while level.front().is_some_and(|order| order.quantity == 0) {
                        level.pop_front();
                    }
                }
                Policy::ProRata => level.retain(|order| order.quantity > 0),
            }
            if level.is_empty() {
                opposite.remove(&price);
            }
        }

        if incoming.quantity > 0 {
            let book = match incoming.side {
                Side::Buy => &mut self.bids,
                Side::Sell => &mut self.asks,
            };
            book.entry(incoming.price).or_default().push_back(incoming);
            self.active
                .insert(incoming.id, (incoming.side, incoming.price));
            events.push(Event::Rested {
                id: incoming.id,
                remaining: incoming.quantity,
            });
        }
        Ok(())
    }

    fn cancel(&mut self, id: u64, events: &mut Vec<Event>) -> Result<(), EngineError> {
        let Some((side, price)) = self.active.get(&id).copied() else {
            events.push(Event::Rejected {
                id,
                reason: "order is not resting",
            });
            return Ok(());
        };
        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = book.get_mut(&price).ok_or(EngineError::CorruptBook)?;
        let index = level
            .iter()
            .position(|order| order.id == id)
            .ok_or(EngineError::CorruptBook)?;
        level.remove(index);
        if level.is_empty() {
            book.remove(&price);
        }
        self.active.remove(&id);
        events.push(Event::Cancelled { id });
        Ok(())
    }

    #[must_use]
    pub fn snapshot(&self) -> BookSnapshot {
        let level = |(&price, orders): (&u64, &VecDeque<Order>)| LevelSnapshot {
            price,
            orders: orders
                .iter()
                .map(|order| (order.id, order.quantity))
                .collect(),
        };
        BookSnapshot {
            bids: self.bids.iter().rev().map(level).collect(),
            asks: self.asks.iter().map(level).collect(),
        }
    }
}

/// Allocations at one price level, in the order trade events will be emitted.
fn allocate(
    policy: Policy,
    level: &VecDeque<Order>,
    requested: u64,
    allocations: &mut Vec<(usize, u64)>,
) -> Result<(), EngineError> {
    allocations.clear();

    match policy {
        Policy::Fifo => {
            let mut remaining = requested;
            for (index, order) in level.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                let quantity = remaining.min(order.quantity);
                if quantity > 0 {
                    allocations.push((index, quantity));
                }
                remaining = remaining
                    .checked_sub(quantity)
                    .ok_or(EngineError::CorruptBook)?;
            }
        }
        Policy::ProRata => {
            let total = level.iter().try_fold(0_u128, |sum, order| {
                sum.checked_add(u128::from(order.quantity))
                    .ok_or(EngineError::ArithmeticOverflow)
            })?;
            if total == 0 {
                return Err(EngineError::CorruptBook);
            }
            let fill = requested.min(u64::try_from(total).unwrap_or(u64::MAX));
            let mut allocated = 0_u64;
            for (index, order) in level.iter().enumerate() {
                let share = u128::from(fill)
                    .checked_mul(u128::from(order.quantity))
                    .and_then(|product| product.checked_div(total))
                    .ok_or(EngineError::ArithmeticOverflow)?;
                let share = u64::try_from(share).map_err(|_| EngineError::ArithmeticOverflow)?;
                allocated = allocated
                    .checked_add(share)
                    .ok_or(EngineError::ArithmeticOverflow)?;
                allocations.push((index, share));
            }
            let mut leftover = fill
                .checked_sub(allocated)
                .ok_or(EngineError::CorruptBook)?;
            // Integer-lot remainder goes to older eligible orders first.
            for ((_, share), order) in allocations.iter_mut().zip(level) {
                if leftover > 0 && *share < order.quantity {
                    *share = share
                        .checked_add(1)
                        .ok_or(EngineError::ArithmeticOverflow)?;
                    leftover = leftover.checked_sub(1).ok_or(EngineError::CorruptBook)?;
                }
            }
            if leftover != 0 {
                return Err(EngineError::CorruptBook);
            }
        }
    }
    Ok(())
}
