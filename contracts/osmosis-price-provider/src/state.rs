use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, IbcChannel};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
}

#[cw_serde]
pub struct Subscription {
    denom: String,
    channel: IbcChannel,
}
