use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Timestamp};

#[cw_serde]
pub struct TradingPair {
    pub pool_id: u64,
    pub base_asset: String,
    pub quote_asset: String,
}

#[cw_serde]
pub struct PriceInfo {
    pub time: Timestamp,
    pub native_per_foreign: Decimal,
}
