//! Integer ticks and lots are the units of this toy market.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Order {
    pub id: u64,
    pub side: Side,
    pub price: u64,
    pub quantity: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Place(Order),
    Cancel(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Policy {
    Fifo,
    ProRata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Trade {
        maker: u64,
        taker: u64,
        price: u64,
        quantity: u64,
    },
    Rested {
        id: u64,
        remaining: u64,
    },
    Cancelled {
        id: u64,
    },
    Rejected {
        id: u64,
        reason: &'static str,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelSnapshot {
    pub price: u64,
    pub orders: Vec<(u64, u64)>, // (order ID, remaining lots), in arrival order
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookSnapshot {
    pub bids: Vec<LevelSnapshot>, // highest price first
    pub asks: Vec<LevelSnapshot>, // lowest price first
}
