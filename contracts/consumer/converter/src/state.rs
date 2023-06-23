use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal};

#[cw_serde]
pub struct Config {
    /// Adjustment to apply on top of the price feed.
    /// Adjustment of 1.0 means take normalized price.
    /// Adjustment of 0.0 means the foreign asset has no value.
    /// Adjustment of 0.4 means the foreign asset has 40% of value after conversion.
    /// Note this is (1.0 - discount)
    pub price_adjustment: Decimal,

    /// Address of the contract we query for the price feed to normalize the foreign asset into native tokens.
    pub price_feed: Addr,

    /// Staking denom used on this chain
    pub local_denom: String,

    /// Token being "virtually sent" over IBC.
    /// use remote via, eg "uosmo", not "ibc/4EF183..."
    pub remote_denom: String,
}
