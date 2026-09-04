pub struct Order {
    id: u64,
    side: Side,
    amount: Quantity,
    at_price: Price,
}

pub struct Price(u64);
pub struct Quantity(u64);
pub enum Side {
    BUY,
    SELL,
}
