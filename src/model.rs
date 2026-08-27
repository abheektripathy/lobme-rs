//!define all the structs required for the engine

pub struct Price(u64);
pub struct Amount(u64);

pub struct PriceLevel {
    level: u64,
}

pub struct OrderId(u64);

pub enum Side {
    Buy,
    Sell,
}

pub enum Assets {
    BTC,
    ETH,
    LIT,
    HYPE,
}

pub struct RestingOrder {
    side: Side,
    price: Price,
    amount: Amount,
    id: OrderId,
    asset: Assets,
}
