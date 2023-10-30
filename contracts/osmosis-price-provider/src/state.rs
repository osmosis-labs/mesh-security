use std::hash::Hash;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, IbcChannel, Uint64};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
}
