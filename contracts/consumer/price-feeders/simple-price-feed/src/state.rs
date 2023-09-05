use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal};

#[cw_serde]
pub struct Config {
    /// Owner who can update price
    pub owner: Addr,

    /// The current set price
    pub native_per_foreign: Decimal,
}
