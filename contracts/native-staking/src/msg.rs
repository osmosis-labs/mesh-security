use crate::state::Config;
use cosmwasm_schema::cw_serde;

pub type ConfigResponse = Config;

#[cw_serde]
pub struct ProxyByOwnerResponse {
    pub proxy: String,
}

#[cw_serde]
pub struct OwnerByProxyResponse {
    pub owner: String,
}

/// The message that is binary encoded in `receive_stake(..msg)`
#[cw_serde]
pub struct StakeMsg {
    pub validator: String,
}

/// The message that is binary encoded in a proxy contract's `Instantiate` message's data
#[cw_serde]
pub struct OwnerMsg {
    pub owner: String,
}
