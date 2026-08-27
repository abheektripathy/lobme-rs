//! core engine logic

use crate::model::{OrderId, Price, PriceLevel, RestingOrder};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Engine {
    asks: BTreeMap<Price, PriceLevel>,
    bids: BTreeMap<Price, PriceLevel>,
    orders: HashMap<OrderId, RestingOrder>,
    seen_ids: HashSet<OrderId>,
}

impl Engine {
    pub fn accept(self) {}
}
